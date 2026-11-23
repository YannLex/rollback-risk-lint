"""Exercise native release binaries with Java/Python/Go revision analysis."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    env = os.environ.copy()
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    with tempfile.TemporaryDirectory(prefix="rrl-smoke-") as temporary:
        root = pathlib.Path(temporary)

        def git(*args):
            return subprocess.check_output(
                ["git", "-C", str(root), *args], env=env, stderr=subprocess.PIPE,
                text=True,
            ).strip()

        git("init", "-b", "main")
        git("config", "user.name", "Smoke Test")
        git("config", "user.email", "smoke@example.invalid")
        git("config", "core.hooksPath", str(root / ".disabled-hooks"))
        policy = {"version": 1, "effects": [
            {"name": "replace", "receiver": "gateway", "method": "replace", "kind": "persistence"},
            {"name": "replace-go", "receiver": "gateway", "method": "Replace", "kind": "persistence"},
        ]}
        (root / ".rollback-risk-lint.json").write_text(json.dumps(policy), encoding="utf-8")
        sources = {
            "Worker.java": "class Worker { void process(Order order) { order.amount = 1; gateway.replace(order); } }\n",
            "worker.py": "def process(order):\n    order.amount = 1\n    gateway.replace(order)\n",
            "worker.go": "package worker\nfunc process(order *Order) {\norder.Amount = 1\ngateway.Replace(order)\n}\n",
        }
        for path, source in sources.items():
            (root / path).write_text(source, encoding="utf-8")
        git("add", ".")
        git("commit", "-m", "before")
        base = git("rev-parse", "HEAD")
        for path, source in sources.items():
            (root / path).write_text(source.replace("= 1", "= 2"), encoding="utf-8")
        git("add", ".")
        git("commit", "-m", "after")
        output = subprocess.run(
            [binary, "--repo", str(root), "--base", base, "--head", "HEAD",
             "--format", "json", "--fail-on", "review"],
            capture_output=True, text=True, env=env, check=False,
        )
        assert output.returncode == 1, output.stderr
        report = json.loads(output.stdout)
        functions = [f for f in report["findings"] if f["rule_id"] == "RRL010"]
        assert {f["path"] for f in functions} == set(sources), report
        assert len(functions) == 3
        assert all(f["boundary"]["source"] == "configured" for f in functions)
        assert report["policy_source"] == "base"
        assert not report["function_analysis"]["warnings"]
        print("Native binary: trusted-policy and Java/Python/Go function review passed.")


if __name__ == "__main__":
    main()
