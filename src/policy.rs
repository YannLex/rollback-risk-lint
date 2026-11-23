use regex::Regex;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Persistence,
    Message,
    Http,
    Payment,
}
impl EffectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Persistence => "persistence",
            Self::Message => "message",
            Self::Http => "http",
            Self::Payment => "payment",
        }
    }
    pub fn rule(self) -> &'static str {
        if matches!(self, Self::Persistence) {
            "RRL004"
        } else {
            "RRL005"
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallPattern {
    pub receiver: String,
    pub method: String,
    #[serde(default)]
    pub path_suffix: String,
}
impl CallPattern {
    fn matches(&self, path: &str, receiver: &str, method: &str) -> bool {
        path.ends_with(&self.path_suffix)
            && method == self.method
            && (self.receiver == "*"
                || receiver == self.receiver
                || (!self.receiver.is_empty()
                    && receiver.ends_with(&format!(".{}", self.receiver))))
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRule {
    pub name: String,
    pub receiver: String,
    pub method: String,
    pub kind: EffectKind,
    #[serde(default)]
    pub path_suffix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    #[serde(default)]
    pub effects: Vec<EffectRule>,
    #[serde(default)]
    pub pure: Vec<CallPattern>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            version: 1,
            effects: Vec::new(),
            pure: Vec::new(),
        }
    }
}
impl Policy {
    pub fn parse(text: &str) -> Result<Self, String> {
        let policy: Self =
            serde_json::from_str(text).map_err(|e| format!("Invalid policy JSON: {e}"))?;
        if policy.version != 1 {
            return Err("Unsupported policy version; expected 1".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for effect in &policy.effects {
            if effect.name.trim().is_empty()
                || !names.insert(&effect.name)
                || effect.method.trim().is_empty()
            {
                return Err(
                    "Effect names must be nonempty and unique; methods must be nonempty".into(),
                );
            }
        }
        if policy.pure.iter().any(|p| p.method.trim().is_empty()) {
            return Err("Pure-call methods must be nonempty".into());
        }
        Ok(policy)
    }
}

#[derive(Clone)]
pub struct MatchedEffect {
    pub source: &'static str,
    pub kind: EffectKind,
    pub name: Option<String>,
}

pub struct Matcher<'a> {
    policy: &'a Policy,
    camel: Regex,
    acronym: Regex,
    storage: Regex,
    external: Regex,
    payment: Regex,
    local_type: Regex,
}
impl<'a> Matcher<'a> {
    pub fn new(policy: &'a Policy) -> Self {
        let re = |s| Regex::new(s).unwrap();
        Self {
            policy,
            camel: re(r"([a-z0-9])([A-Z])"),
            acronym: re(r"([A-Z])([A-Z][a-z])"),
            storage: re(
                r"(?i)(?:^|_)(db|dao|repo|repository|collection|table|storage|store|session|statement|connection|entity_manager|persistence)(?:_|$)",
            ),
            external: re(
                r"(?i)(?:^|_)(client|http|rest|transport|request|producer|publisher|broker|sender|bridge|writer|bus|mail|smtp|payment|socket|topic|channel)(?:_|$)",
            ),
            local_type: re(
                r"\b(?:Map|HashMap|LinkedHashMap|TreeMap|ConcurrentHashMap|ByteBuffer|CRC32|ObjectNode|ArrayNode)(?:<[^;\n=]+>)?\s+(?P<name>[\w$]+)\s*(?:=|;)",
            ),
            payment: re(
                r"(?i)(?:^|_)(payment|payments|stripe|checkout|billing|gateway|bank|transaction)(?:_|$)",
            ),
        }
    }
    pub fn classify_in(
        &self,
        path: &str,
        receiver: &str,
        method: &str,
        context: &str,
    ) -> Option<MatchedEffect> {
        let effect = self.classify(path, receiver, method)?;
        if effect.source == "heuristic" {
            let leaf = receiver.rsplit('.').next().unwrap_or(receiver);
            if self
                .local_type
                .captures_iter(context)
                .any(|c| &c["name"] == leaf)
            {
                return None;
            }
        }
        Some(effect)
    }
    /// Configured effects take precedence over pure declarations and name heuristics.
    pub fn classify(&self, path: &str, receiver: &str, method: &str) -> Option<MatchedEffect> {
        for rule in &self.policy.effects {
            let pattern = CallPattern {
                receiver: rule.receiver.clone(),
                method: rule.method.clone(),
                path_suffix: rule.path_suffix.clone(),
            };
            if pattern.matches(path, receiver, method) {
                return Some(MatchedEffect {
                    source: "configured",
                    kind: rule.kind,
                    name: Some(rule.name.clone()),
                });
            }
        }
        if self
            .policy
            .pure
            .iter()
            .any(|p| p.matches(path, receiver, method))
        {
            return None;
        }
        let leaf = receiver.rsplit('.').next().unwrap_or(receiver);
        let normalized = self.acronym.replace_all(leaf, "${1}_${2}");
        let normalized = self
            .camel
            .replace_all(&normalized, "${1}_${2}")
            .to_ascii_lowercase();
        let method = method.to_ascii_lowercase();
        let kind = if method.starts_with("save")
            || method.starts_with("insert")
            || (self.storage.is_match(&normalized)
                && ["update", "delete", "remove", "exec", "write"]
                    .iter()
                    .any(|s| method.starts_with(s)))
        {
            EffectKind::Persistence
        } else if self.payment.is_match(&normalized)
            && ["capture", "charge", "refund"].contains(&method.as_str())
        {
            EffectKind::Payment
        } else if self.external.is_match(&normalized)
            && ["post", "put", "patch"].contains(&method.as_str())
        {
            EffectKind::Http
        } else if self.external.is_match(&normalized)
            && ["publish", "send", "produce"].contains(&method.as_str())
        {
            EffectKind::Message
        } else {
            return None;
        };
        Some(MatchedEffect {
            source: "heuristic",
            kind,
            name: None,
        })
    }
}
