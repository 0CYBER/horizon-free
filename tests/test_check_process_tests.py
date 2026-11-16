"""tools/ci/check_process_tests.py: an unignored test may not start a child process."""
import subprocess
import sys
import textwrap
import unittest
from pathlib import Path

from tools.ci import check_process_tests as check

ROOT = Path(__file__).resolve().parents[1]


def names(source, listed=()):
    return [(function.name, chain) for function, chain in check.offenders_in(textwrap.dedent(source), list(listed))]


class ProcessTests(unittest.TestCase):
    def test_a_test_that_runs_a_command_or_reaches_one_through_a_helper_is_named(self):
        source = """\
            use std::process::Command;
            fn tool() -> Command { Command::new(env!("CARGO_BIN_EXE_tool")) }
            fn run(args: &[&str]) -> String { String::from_utf8(tool().args(args).output().unwrap().stdout).unwrap() }
            struct Scene;
            impl Scene { fn start() -> Self { let _ = std::process::Command::new("sh").spawn(); Scene } }
            #[test] fn direct() { assert!(Command::new("true").status().unwrap().success()); }
            #[test] fn helper() { run(&["new"]); }
            #[test] fn through_a_type() { Scene::start(); }
            #[test] fn builds_only() { let _ = tool(); }
            #[test] #[ignore = "daily"] fn moved() { run(&[]); }
            #[cfg(windows)] #[test] fn windows_only() { run(&[]); }
            #[cfg(target_os = "windows")] #[test] fn windows_by_name() { run(&[]); }
            """
        self.assertEqual(names(source), [("direct", "runs a Command"), ("helper", "run -> runs a Command"),
                                         ("through_a_type", "start -> runs a Command")])

    def test_threads_and_other_status_methods_are_not_processes(self):
        source = """\
            fn spawn() -> u8 { let _ = std::process::Command::new("x").spawn(); 1 }
            #[test] fn a_thread() { std::thread::spawn(move || 1).join().unwrap(); }
            #[test] fn a_builder_thread() { std::thread::Builder::new().spawn(|| 1).unwrap(); }
            #[test] fn an_effect() { let effects = Effects::default(); assert!(effects.status().is_empty()); }
            """
        self.assertEqual(names(source), [])

    def test_a_listed_helper_and_its_alias_start_a_process(self):
        source = """\
            use crate::runtime_leases::isolated_test as isolated;
            #[test] fn by_alias() { if isolated() { return; } }
            #[test] fn by_path() { horizon_mod_wasm::fixture::cached_answer(&root, exe, &bytes); }
            #[test] fn unrelated() { other::answer(); }
            """
        self.assertEqual(names(source, ["isolated_test", "fixture::cached_answer"]),
                         [("by_alias", "isolated"), ("by_path", "fixture::cached_answer")])

    def test_the_committed_tree_has_no_process_starting_unit_test(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/ci/check_process_tests.py")],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout[-2000:])
        self.assertIn("0 unignored tests start a process", result.stdout)


if __name__ == "__main__":
    unittest.main()
