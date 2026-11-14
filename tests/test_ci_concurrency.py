"""Pin the actual parsed workflow's main queue and PR replacement behavior."""
import ast
from pathlib import Path
import re
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
# Use the same offline parser as the repository guard, not a runner package.
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml

# The daily run's legs: called by daily.yml or dispatched by hand, never on pull requests or main pushes (owner,
# 2026-11-12; CI tiers 2026-11-13).
DAILY_LEGS = [ROOT / ".github/workflows" / name for name in ("wasm-mods.yml", "fn-replace-check.yml",
                                                                          "machine-sample.yml")]


def expression_value(expression, github, inputs=None, matrix=None):
    """Evaluate this workflow's context/equality/boolean subset; reject other syntax.

    These contexts use nonzero PR numbers, lowercase event/ref strings and nonempty
    SHAs, whose truthiness/equality matches Actions. Boolean operators return their
    operands, so the group's fallback chain is exercised rather than pinned as text.
    """
    def evaluate(node):
        if isinstance(node, ast.Constant):
            return node.value
        if isinstance(node, ast.Name):
            return {"github": github, "inputs": inputs or {}, "matrix": matrix or {},
                    "true": True, "false": False}[node.id]
        if isinstance(node, ast.Attribute):
            return evaluate(node.value).get(node.attr, "")
        if isinstance(node, ast.Compare) and len(node.ops) == 1 and isinstance(node.ops[0], (ast.Eq, ast.NotEq)):
            equal = evaluate(node.left) == evaluate(node.comparators[0])
            return equal if isinstance(node.ops[0], ast.Eq) else not equal
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "format":
            template, *values = [evaluate(arg) for arg in node.args]
            for index, value in enumerate(values):
                template = template.replace(f"{{{index}}}", str(value))
            return template
        if isinstance(node, ast.BoolOp):
            for value in node.values:
                result = evaluate(value)
                if isinstance(node.op, ast.Or) and result:
                    return result
                if isinstance(node.op, ast.And) and not result:
                    return result
            return result
        raise AssertionError(f"unsupported Actions expression syntax: {ast.dump(node)}")

    return evaluate(ast.parse(expression.replace("&&", " and ").replace("||", " or ").strip(), mode="eval").body)


def concurrency_for(path, event, sha, number=None, ref="refs/heads/main", inputs=None, job=None,
                    matrix=None, run_id="1"):
    workflow = yaml.safe_load(path.read_text())
    policy = workflow["jobs"][job]["concurrency"] if job else workflow["concurrency"]
    github = {"event_name": event, "sha": sha, "ref": ref, "run_id": run_id,
              "event": {"pull_request": {"number": number or ""}}}
    if number is not None:
        github["ref"] = f"refs/pull/{number}/merge"
    group = re.sub(r"\$\{\{(.*?)\}\}",
                   lambda match: str(expression_value(match.group(1), github, inputs, matrix)), policy["group"])
    cancel = policy["cancel-in-progress"]
    if isinstance(cancel, str):
        match = re.fullmatch(r"\$\{\{(.*?)\}\}", cancel)
        if match is None:
            raise AssertionError("cancellation must be a boolean or an Actions expression")
        cancel = expression_value(match.group(1), github, inputs, matrix)
    if not isinstance(cancel, bool):
        raise AssertionError("cancellation did not evaluate to a boolean")
    return group, cancel


class ConcurrencyTests(unittest.TestCase):
    def test_daily_legs_never_run_on_pull_requests_or_pushes(self):
        for path in DAILY_LEGS + [ROOT / ".github/workflows" / name
                                  for name in ("ignored-tests.yml", "native-dylib-probe.yml")]:
            with self.subTest(workflow=path.name):
                triggers = yaml.load(path.read_text(), Loader=yaml.BaseLoader)["on"]
                self.assertEqual(set(triggers), {"workflow_call", "workflow_dispatch"})
                self.assertEqual(set(triggers["workflow_call"]["inputs"]), {"commit"})

    def test_daily_calls_finish_and_hand_dispatches_keep_apart(self):
        for path in DAILY_LEGS:
            with self.subTest(workflow=path.name):
                daily = concurrency_for(path, "workflow_dispatch", "a" * 40, ref="refs/heads/main",
                                        inputs={"commit": "a" * 40}, run_id="10")
                daily_branch = concurrency_for(path, "workflow_dispatch", "a" * 40, ref="refs/heads/suspect",
                                               inputs={"commit": "a" * 40}, run_id="11")
                main_dispatch = concurrency_for(path, "workflow_dispatch", "b" * 40, run_id="12")
                main_again = concurrency_for(path, "workflow_dispatch", "b" * 40, run_id="13")
                branch = concurrency_for(path, "workflow_dispatch", "c" * 40, ref="refs/heads/suspect")
                branch_again = concurrency_for(path, "workflow_dispatch", "d" * 40, ref="refs/heads/suspect")
                other_branch = concurrency_for(path, "workflow_dispatch", "c" * 40, ref="refs/heads/other")
                for _, cancel in (daily, daily_branch, main_dispatch, main_again):
                    self.assertIs(cancel, False, "a daily leg and a main dispatch always finish")
                self.assertNotIn(daily_branch[0], (branch[0], daily[0]),
                                 "a hand dispatch on a branch never displaces a daily leg")
                self.assertNotEqual(main_dispatch[0], main_again[0], "a dispatch on main is never folded into another")
                self.assertEqual(branch[0], branch_again[0], "a newer dispatch on a branch replaces the older")
                self.assertIs(branch_again[1], True)
                self.assertNotEqual(other_branch[0], branch[0], "other refs never displace each other")

if __name__ == "__main__":
    unittest.main()
