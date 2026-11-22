"""A failed compile in a harness shows the end of its log, so the parity run's artifact holds the cause."""
import contextlib
import importlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

harness = importlib.import_module('tools.oracle_harness')


class RunLoggedTest(unittest.TestCase):
    def run_fake(self, script, arguments=('build',), program='cargo', name='compile-rust', error=None, **kwargs):
        """run_logged over a fake program named `program` (given `arguments`) that runs `script`. Returns what it
        printed, its log and the exception it raised (expected to be `error`, None for a pass)."""
        with tempfile.TemporaryDirectory() as temporary:
            build = Path(temporary)
            fake = build / program
            fake.write_text(script, encoding='utf-8')
            real = subprocess.run
            stderr = io.StringIO()
            raised = None
            with contextlib.redirect_stderr(stderr), \
                    mock.patch.object(harness.subprocess, 'run',
                                      lambda command, **options: real([sys.executable, str(fake)], **options)):
                try:
                    harness.run_logged([str(fake), *arguments], build, name, **kwargs)
                except Exception as exception:
                    raised = exception
            if error is None:
                self.assertIsNone(raised)
            else:
                self.assertIsInstance(raised, error)
            return stderr.getvalue(), (build / (name + '.log')).read_text(encoding='utf-8'), raised

    def test_a_failed_compiles_log_tail_is_printed(self):
        printed, log, raised = self.run_fake("import sys\nfor i in range(60): print('line', i)\nsys.exit(101)",
                                             error=subprocess.CalledProcessError)
        self.assertEqual(raised.returncode, 101)
        self.assertIn('line 59', log)
        self.assertIn('--- last 40 lines of compile-rust.log ---', printed)
        self.assertIn('line 59', printed)
        self.assertNotIn('line 19\n', printed)

    def test_cargo_json_messages_print_as_cargo_would_show_errors(self):
        error = {'reason': 'compiler-message', 'message': {'level': 'error', 'rendered': 'error[E0425]: cannot find value'}}
        warning = {'reason': 'compiler-message', 'message': {'level': 'warning', 'rendered': 'warning: unused'}}
        artifact = {'reason': 'compiler-artifact', 'filenames': ['/x']}
        odd = {'message': 'failed'}  # not cargo's shape: skipped, and never replaces the command's own error
        lines = [json.dumps(warning), json.dumps(artifact), json.dumps(odd), json.dumps(error)]
        script = ("import sys\n" + "".join(f"print({line!r})\n" for line in lines)
                  + "print('error: could not compile `horizon-client`', file=sys.stderr)\nsys.exit(101)")
        printed, _, _ = self.run_fake(script, ('test', '--no-run', '--message-format=json'),
                                      error=subprocess.CalledProcessError)
        self.assertIn('error[E0425]: cannot find value', printed)
        self.assertIn('error: could not compile `horizon-client`', printed)
        self.assertNotIn('warning: unused', printed)
        self.assertNotIn('compiler-artifact', printed)
        self.assertNotIn('failed', printed.replace('could not compile', ''))

    def test_the_checkout_and_home_folder_are_not_shown_and_long_lines_are_cut(self):
        checkout, home = str(harness.lab.ROOT), str(Path.home())
        script = (f"import sys\nprint({checkout + '/src/lib.rs'!r})\nprint({home + '/.cargo/x'!r})\n"
                  "print('x' * 5000)\nsys.exit(101)")
        printed, _, _ = self.run_fake(script, error=subprocess.CalledProcessError)
        self.assertIn('<checkout>/src/lib.rs', printed)
        self.assertIn('~/.cargo/x', printed)
        self.assertNotIn(checkout, printed)
        self.assertIn('x' * harness.LOG_LINE_CHARS, printed)
        self.assertNotIn('x' * (harness.LOG_LINE_CHARS + 1), printed)

    def test_a_record_cut_by_the_read_window_is_not_shown(self):
        record = json.dumps({'reason': 'compiler-artifact', 'filenames': ['secret-looking-path' * 20]})
        script = f"import sys\nprint({record!r})\nprint('error: could not compile')\nsys.exit(101)"
        with mock.patch.object(harness, 'LOG_TAIL_BYTES', 100):
            printed, _, _ = self.run_fake(script, error=subprocess.CalledProcessError)
        self.assertIn('error: could not compile', printed)
        self.assertNotIn('secret-looking-path', printed)

    def test_a_timed_out_compile_is_reported_and_keeps_its_error(self):
        printed, _, _ = self.run_fake("import time\nprint('compiling', flush=True)\ntime.sleep(30)",
                                      error=subprocess.TimeoutExpired, timeout=2)
        self.assertIn('compiling', printed)

    def test_what_runs_a_program_is_never_printed(self):
        # A test run, an oracle or the game can print vanilla game data: it stays in the runner's log file.
        for program, arguments in [('cargo', ('test', '-p', 'horizon-client', '--', '--nocapture')),
                                   ('cargo', ('run', '-p', 'probe')), ('java', ('-cp', 'x', 'Oracle'))]:
            with self.subTest(program=program, arguments=arguments):
                printed, log, _ = self.run_fake("import sys\nprint('vanilla data')\nsys.exit(1)", arguments, program,
                                                error=subprocess.CalledProcessError)
                self.assertIn('vanilla data', log)
                self.assertEqual(printed, '')

    def test_a_compile_that_passes_prints_nothing(self):
        printed, log, _ = self.run_fake("print('Finished')", ('test', '--no-run'))
        self.assertEqual(log.strip(), 'Finished')
        self.assertEqual(printed, '')

    def test_a_failing_report_never_replaces_the_commands_error(self):
        with mock.patch.object(harness, 'print_log_tail', side_effect=OSError('unreadable')):
            _, _, raised = self.run_fake("import sys\nsys.exit(101)", error=subprocess.CalledProcessError)
        self.assertEqual(raised.returncode, 101)


if __name__ == '__main__':
    unittest.main()
