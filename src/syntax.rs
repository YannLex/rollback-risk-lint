use tree_sitter::{Node, Parser};

#[derive(Debug)]
pub(crate) struct Call {
    pub receiver: String,
    pub method: String,
    pub line: usize,
    pub end: usize,
    pub text: String,
    pub tokens: Vec<String>,
}
#[derive(Debug)]
pub(crate) struct Function {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub tokens: Vec<String>,
    pub calls: Vec<Call>,
}

pub(crate) fn supported(path: &str) -> bool {
    ["java", "py", "go"].contains(&path.rsplit('.').next().unwrap_or(""))
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn is_function(node: Node<'_>) -> bool {
    [
        "method_declaration",
        "constructor_declaration",
        "function_declaration",
        "function_definition",
    ]
    .contains(&node.kind())
}
fn contains_call(node: Node<'_>) -> bool {
    if ["method_invocation", "call", "call_expression"].contains(&node.kind()) {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(contains_call)
}
fn logging(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "expression_statement" {
        return false;
    }
    if let Some(call) = node.named_child(0) {
        let mut cursor = call.walk();
        if call.named_children(&mut cursor).any(contains_call) {
            return false;
        }
    }
    let target: String = text(node, source)
        .split('(')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    ["log", "logger", "LOGGER"].iter().any(|receiver| {
        ["info", "debug", "warn", "error", "trace", "fatal", "log"]
            .iter()
            .any(|method| target == format!("{receiver}.{method}"))
    })
}
fn docstring(node: Node<'_>) -> bool {
    node.kind() == "expression_statement"
        && node
            .named_child(0)
            .is_some_and(|n| n.kind() == "string" && !contains_call(n))
        && node
            .parent()
            .is_some_and(|p| p.kind() == "block" && p.named_child(0) == Some(node))
}
fn tokens(node: Node<'_>, source: &str, root: bool, output: &mut Vec<String>) {
    if node.kind().contains("comment")
        || logging(node, source)
        || docstring(node)
        || (!root && is_function(node))
    {
        return;
    }
    if node.child_count() == 0 {
        let value = text(node, source);
        if !value.is_empty() {
            output.push(value.to_owned());
        }
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            tokens(child, source, false, output);
        }
    }
}
fn call(node: Node<'_>, source: &str) -> Option<Call> {
    let (receiver, method) = if node.kind() == "method_invocation" {
        (
            node.child_by_field_name("object")
                .map(|n| text(n, source))
                .unwrap_or(""),
            text(node.child_by_field_name("name")?, source),
        )
    } else if ["call", "call_expression"].contains(&node.kind()) {
        let function = node.child_by_field_name("function")?;
        if function.kind() == "attribute" {
            (
                text(function.child_by_field_name("object")?, source),
                text(function.child_by_field_name("attribute")?, source),
            )
        } else if function.kind() == "selector_expression" {
            (
                text(function.child_by_field_name("operand")?, source),
                text(function.child_by_field_name("field")?, source),
            )
        } else if function.kind() == "identifier" {
            ("", text(function, source))
        } else {
            return None;
        }
    } else {
        return None;
    };
    let mut normalized = Vec::new();
    tokens(node, source, true, &mut normalized);
    Some(Call {
        receiver: receiver.chars().filter(|c| !c.is_whitespace()).collect(),
        method: method.into(),
        line: node.start_position().row + 1,
        end: node.end_position().row + 1,
        text: text(node, source).into(),
        tokens: normalized,
    })
}
fn calls(node: Node<'_>, source: &str, root: bool, output: &mut Vec<Call>) {
    if (!root && is_function(node)) || node.kind().contains("comment") || logging(node, source) {
        return;
    }
    if let Some(call) = call(node, source) {
        output.push(call);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        calls(child, source, false, output);
    }
}
fn collect(node: Node<'_>, source: &str, parents: &[String], output: &mut Vec<Function>) {
    let scope = is_function(node)
        || [
            "class_declaration",
            "interface_declaration",
            "class_definition",
            "record_declaration",
            "enum_declaration",
        ]
        .contains(&node.kind());
    let mut parents = parents.to_vec();
    if scope && let Some(name) = node.child_by_field_name("name") {
        let mut name = text(name, source).to_owned();
        if let Some(parameters) = node.child_by_field_name("parameters")
            && parameters.kind() == "formal_parameters"
        {
            let mut cursor = parameters.walk();
            let types: Vec<_> = parameters
                .named_children(&mut cursor)
                .filter(|p| !p.kind().contains("comment"))
                .map(|p| {
                    let kind = p.child_by_field_name("type").unwrap_or(p);
                    text(kind, source)
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>()
                })
                .collect();
            name = format!("{name}({})", types.join(","));
        }

        // Go method receiver types distinguish the same method on different types.
        if let Some(receiver) = node.child_by_field_name("receiver") {
            let mut cursor = receiver.walk();
            for parameter in receiver.named_children(&mut cursor) {
                if let Some(kind) = parameter.child_by_field_name("type") {
                    name = format!("{}.{}", text(kind, source), name);
                }
            }
        }
        parents.push(name);
    }
    if is_function(node) && node.child_by_field_name("body").is_some() {
        let mut normalized = Vec::new();
        tokens(node, source, true, &mut normalized);
        let mut effect_calls = Vec::new();
        calls(node, source, true, &mut effect_calls);
        output.push(Function {
            name: parents.join("."),
            text: text(node, source).into(),
            start: node.start_position().row + 1,
            end: node.end_position().row + 1,
            tokens: normalized,
            calls: effect_calls,
        });
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, source, &parents, output);
    }
}
pub(crate) fn functions(path: &str, source: &str) -> Result<Vec<Function>, String> {
    let language = match path.rsplit('.').next().unwrap_or("") {
        "java" => tree_sitter_java::LANGUAGE.into(),
        "py" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return Err("Unsupported function-analysis language".into()),
    };
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|e| e.to_string())?;
    let tree = parser
        .parse(source, None)
        .ok_or("Parser did not produce a tree")?;
    if tree.root_node().has_error() {
        return Err("Syntax error; function analysis skipped".into());
    }
    let mut output = Vec::new();
    collect(tree.root_node(), source, &[], &mut output);
    Ok(output)
}
