"""Every Python test that runs `rustc` or `cargo` must skip when the tool is missing.

`verify` can run the Python suite where no Rust toolchain is installed (a documentation-only change, which installs
none), and a contributor may run it without one, so an unguarded `subprocess.run(["rustc", ...])` fails there with
FileNotFoundError (arismoko/horizon#2868, and `tests/test_prebuilt_game.py` after arismoko/horizon#2622).

The rule, checked statically over `tests/*.py`: a test that starts `rustc` or `cargo` (a command list, a local name
bound to one, or a shell string; through `subprocess.<launcher>` or a launcher imported from subprocess), either
itself or through a helper function or method of the same module, carries a guard: `unittest.skipUnless(
shutil.which(...))` (or `skipIf`) on the test or its class, or a `shutil.which(...)` check that skips
(`self.skipTest` or `raise unittest.SkipTest`) in the test, a helper on the way, `setUp`, `setUpClass` or
`setUpModule`. A command list that is only built and compared (mock assertions) is not a launch.
Not followed: launches inside `tools/` code that a test calls.
"""
import ast
from pathlib import Path
import unittest

TESTS = Path(__file__).resolve().parent
TOOLS = {"rustc", "cargo"}
LAUNCHERS = {"run", "check_output", "check_call", "call", "Popen"}


def _str(node):
    return node.value if isinstance(node, ast.Constant) and isinstance(node.value, str) else None


def _which_tool(node):
    """`shutil.which("rustc")` or `which("cargo")`: the tool it looks up, if one of ours."""
    if isinstance(node, ast.Call):
        func = node.func
        name = func.attr if isinstance(func, ast.Attribute) else getattr(func, "id", None)
        if name == "which" and node.args and _str(node.args[0]) in TOOLS:
            return _str(node.args[0])
    return None


def _mentions_which(node):
    return any(_which_tool(n) for n in ast.walk(node))


def _is_guard_decorator(node):
    # unittest.skipUnless(shutil.which("rustc"), ...) / skipIf(shutil.which("cargo") is None, ...)
    if not isinstance(node, ast.Call):
        return False
    func = node.func
    name = func.attr if isinstance(func, ast.Attribute) else getattr(func, "id", None)
    return name in {"skipUnless", "skipIf"} and _mentions_which(node)


def _guarded_body(func):
    """The body checks for the tool and skips: `self.skipTest(...)` or `raise unittest.SkipTest(...)`."""
    def skips(node):
        if not isinstance(node, ast.Call):
            return False
        name = node.func.attr if isinstance(node.func, ast.Attribute) else getattr(node.func, "id", None)
        return name in {"skipTest", "SkipTest"}  # self.skipTest(...) or raise unittest.SkipTest(...)
    has_skip = any(skips(n) for n in ast.walk(func))
    return has_skip and _mentions_which(func)


def _which_names(func):
    """Local names bound to `shutil.which(tool)`: `compiler = shutil.which("rustc")`."""
    names = set()
    for node in ast.walk(func):
        if isinstance(node, ast.Assign) and _which_tool(node.value):
            names |= {t.id for t in node.targets if isinstance(t, ast.Name)}
    return names


def _names_tool(command, which_names, lists):
    """A command that starts rustc or cargo: a list whose first item names one (or a `shutil.which` result), a
    shell string starting with one, or a local name bound to such a list."""
    if isinstance(command, ast.Name) and command.id in lists:
        command = lists[command.id]
    if isinstance(command, (ast.List, ast.Tuple)) and command.elts:
        first = command.elts[0]
        return _str(first) in TOOLS or (isinstance(first, ast.Name) and first.id in which_names)
    text = _str(command)
    return text is not None and text.split(" ", 1)[0] in TOOLS


def _launches(func, imported):
    """Whether this function body itself starts rustc or cargo through subprocess (`subprocess.run(...)`, or
    `run(...)` imported from subprocess)."""
    which_names = _which_names(func)
    lists = {t.id: node.value for node in ast.walk(func) if isinstance(node, ast.Assign)
             and isinstance(node.value, (ast.List, ast.Tuple)) for t in node.targets if isinstance(t, ast.Name)}
    for node in ast.walk(func):
        if not (isinstance(node, ast.Call) and node.args):
            continue
        func_node = node.func
        launcher = (isinstance(func_node, ast.Attribute) and func_node.attr in LAUNCHERS) or \
            (isinstance(func_node, ast.Name) and func_node.id in imported)
        if launcher and _names_tool(node.args[0], which_names, lists):
            return True
    return False


def _called_names(func):
    names = set()
    for node in ast.walk(func):
        if isinstance(node, ast.Call):
            if isinstance(node.func, ast.Name):
                names.add(node.func.id)
            elif isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name) \
                    and node.func.value.id in {"self", "cls"}:
                names.add(node.func.attr)
    return names


def unguarded(source, filename="<test>"):
    """Test names in this module that can launch rustc or cargo without a guard."""
    tree = ast.parse(source, filename)
    imported = {alias.asname or alias.name for node in ast.walk(tree)
                if isinstance(node, ast.ImportFrom) and node.module == "subprocess"
                for alias in node.names if alias.name in LAUNCHERS}
    module_guarded = any(isinstance(n, ast.FunctionDef) and n.name == "setUpModule" and _guarded_body(n)
                         for n in tree.body)
    # Every function and method, with its owning class (None for module level).
    functions = []
    for node in tree.body:
        if isinstance(node, ast.FunctionDef):
            functions.append((None, node))
        elif isinstance(node, ast.ClassDef):
            functions += [(node, n) for n in node.body if isinstance(n, ast.FunctionDef)]
    by_name = {}
    for owner, func in functions:
        by_name.setdefault(func.name, []).append((owner, func))

    def guarded(owner, func):
        return (module_guarded or _guarded_body(func) or any(_is_guard_decorator(d) for d in func.decorator_list)
                or (owner is not None and any(_is_guard_decorator(d) for d in owner.decorator_list))
                or (owner is not None and any(n.name in {"setUp", "setUpClass"} and _guarded_body(n)
                                              for n in owner.body if isinstance(n, ast.FunctionDef))))

    # A function launches unguarded if it launches itself without a guard, or calls (within this module) a
    # function that does; a guard anywhere on the way protects the call.
    memo = {}

    def unguarded_launch(owner, func, seen=()):
        key = (owner.name if owner else None, func.name)
        if key in memo:
            return memo[key]
        if key in seen or guarded(owner, func):
            return False
        result = _launches(func, imported) or any(
            unguarded_launch(o, f, seen + (key,)) for name in _called_names(func) for o, f in by_name.get(name, []))
        memo[key] = result
        return result

    return sorted(f"{owner.name}.{func.name}" if owner else func.name
                  for owner, func in functions if func.name.startswith("test") and unguarded_launch(owner, func))


class ToolchainGuardRuleTests(unittest.TestCase):
    def test_every_test_that_starts_rustc_or_cargo_skips_without_it(self):
        offenders = []
        for path in sorted(TESTS.glob("*.py")):
            offenders += [f"{path.name}: {name}" for name in unguarded(path.read_text(encoding="utf-8"), str(path))]
        self.assertEqual([], offenders, "guard these with shutil.which(...) and a skip; see this module's docstring")

    def test_an_unguarded_launch_is_found_directly_and_through_a_helper(self):
        direct = 'import subprocess, unittest\nclass T(unittest.TestCase):\n' \
                 '    def test_a(self):\n        subprocess.run(["rustc", "x.rs"], check=True)\n'
        self.assertEqual(["T.test_a"], unguarded(direct))
        helper = 'import subprocess, unittest\ndef build():\n    subprocess.check_output(["cargo", "build"])\n' \
                 'class T(unittest.TestCase):\n    def test_b(self):\n        build()\n' \
                 '    def compile(self):\n        subprocess.Popen(["rustc"])\n' \
                 '    def test_c(self):\n        self.compile()\n'
        self.assertEqual(["T.test_b", "T.test_c"], unguarded(helper))
        other_forms = 'from subprocess import run\nimport subprocess, unittest\nclass T(unittest.TestCase):\n' \
                      '    def test_imported(self):\n        run(["cargo", "build"])\n' \
                      '    def test_shell(self):\n        subprocess.run("rustc --version", shell=True)\n' \
                      '    def test_variable(self):\n        command = ["cargo", "test"]\n        subprocess.run(command)\n' \
                      '    @classmethod\n    def build(cls):\n        subprocess.run(["cargo", "build"])\n' \
                      '    def test_classmethod(self):\n        self.build()\n'
        self.assertEqual(["T.test_classmethod", "T.test_imported", "T.test_shell", "T.test_variable"],
                         unguarded(other_forms))

    def test_each_guard_form_is_accepted(self):
        guarded = '''
import shutil, subprocess, unittest

@unittest.skipUnless(shutil.which("cargo"), "requires Cargo")
class Whole(unittest.TestCase):
    def test_a(self):
        subprocess.run(["cargo", "build"])

class Parts(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_decorated(self):
        subprocess.run(["rustc"])

    def test_in_body(self):
        compiler = shutil.which("rustc")
        if compiler is None:
            self.skipTest("requires Rust")
        subprocess.run([compiler, "x.rs"])

    def helper(self):
        if not shutil.which("cargo"):
            self.skipTest("requires Cargo")
        subprocess.run(["cargo", "test"])

    def test_through_a_guarded_helper(self):
        self.helper()

class Shared(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if shutil.which("cargo") is None:
            raise unittest.SkipTest("requires Cargo")

    def test_after_class_setup(self):
        subprocess.run(["cargo", "build"])

class Mocked(unittest.TestCase):
    def test_only_builds_the_command(self):
        command = ["cargo", "build"]
        self.assertEqual("cargo", command[0])
'''
        self.assertEqual([], unguarded(guarded))


if __name__ == "__main__":
    unittest.main()
