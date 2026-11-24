"""Synthetic failure payloads only; no game or prepared-data execution."""
import contextlib
import io
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.ci import ignored_tests as ci


class Budgets(unittest.TestCase):
    def setUp(self):
        # A runner invocation's shared budget and cached Git blobs never leak
        # into an independently rendered diagnostic fixture.
        for name, value in (('_active_budget', None), ('_panic_locations', {})):
            patcher = patch.object(ci, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        ci.tracked_rust_paths.cache_clear()
        ci.source_lines.cache_clear()
        self.addCleanup(ci.tracked_rust_paths.cache_clear)
        self.addCleanup(ci.source_lines.cache_clear)

    def render(self, output, *, tracked=None):
        with contextlib.ExitStack() as stack:
            if tracked is not None:
                stack.enter_context(patch.object(ci, 'tracked_rust_paths', return_value=set(tracked)))
                stack.enter_context(patch.object(ci, 'source_lines', side_effect=tracked.get))
            log = stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            ci.failure_diagnostics(output)
        return log.getvalue()

    def test_public_client_excerpt_preserves_reason_despite_all_withheld_counter(self):
        # Public job 130479644435, lines 326–330 (timestamps removed).
        # Raw panic locations/payloads are unavailable; input below is synthetic,
        # not a reconstruction of private output.
        fixture = Path(__file__).with_name('fixtures') / 'client_failure_public_excerpt.txt'
        excerpt = fixture.read_text()
        name = excerpt.splitlines()[0].removeprefix('  failed test: ')
        log = self.render(f"test {name} ... FAILED\n"
                          f"thread '{name}' panicked at private.rs:1:1:\n"
                          "assertion `left == right` failed: PRIVATE\n"
                          ' left: "PRIVATE"\n right: "PRIVATE"\n')
        self.assertEqual(log.split('failure diagnostics: ')[0],
                         '\n'.join(excerpt.splitlines()[:3]) + '\n')
        self.assertIn('1 withheld context/identity', log)
        self.assertIn('1 classified panic reasons; 0 identities withheld; 1 locations withheld; '
                      '1 payloads withheld (expected privacy filtering)', log)
        self.assertNotIn('PRIVATE', log)

    def test_generated_absolute_location_uses_only_this_checkout_root(self):
        root = Path(ci.__file__).resolve().parents[2]
        path = 'crates/fixture/src/lib.rs'
        tracked = {path: ['', '']}
        location = f'{root / path}:2:1'
        log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {location}:\n"
                          "assertion failed: PRIVATE\n", tracked=tracked)
        self.assertIn(f'panicked at {path}:2:1', log)
        self.assertNotIn(str(root), log)
        self.assertNotIn('PRIVATE', log)
        for rejected in (f'/another-checkout/{path}:2:1',
                         f'{root.with_name(root.name + "-other") / path}:2:1',
                         f'{root / ".." / root.name / path}:2:1',
                         f'{Path(root.anchor) / "foreign" / ".." / ".." / path}:2:1',
                         f'{root / path}:3:1',
                         f'{root / path}:2:0',
                         f'{root / "crates/horizon-client/src/not_tracked.rs"}:1:1',
                         f'{root / ".local/private.rs"}:1:1'):
            with self.subTest(location=rejected):
                log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {rejected}:\n"
                                  "assertion failed: PRIVATE\n", tracked=tracked)
                self.assertIn('panic location withheld', log)
                self.assertNotIn('  panicked at ', log)
                self.assertNotIn(str(root), log)

    def test_windows_verbatim_location_uses_only_this_checkout_root(self):
        # game-discovery canonicalizes #[path] declarations: \\?\C:\... on Windows.
        root = Path(ci.__file__).resolve().parents[2]
        path = 'crates/fixture/src/lib.rs'
        tracked = {path: ['', '']}
        windows = (root / path).as_posix().replace('/', '\\')
        with patch.object(ci.os, 'sep', '\\'):
            log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at \\\\?\\{windows}:2:1:\n"
                              "assertion failed: PRIVATE\n", tracked=tracked)
            self.assertIn(f'panicked at {path}:2:1', log)
            log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at "
                              f"\\\\?\\UNC\\host\\share\\{windows}:2:1:\n"
                              "assertion failed: PRIVATE\n", tracked=tracked)
            self.assertIn('panic location withheld', log)

    def test_relative_and_cross_crate_raw_panic_headers(self):
        path = 'crates/fixture/src/lib.rs'
        tracked = {path: ['', '']}
        for raw_path in (path, 'crates/other/../fixture/src/lib.rs'):
            log = self.render(f"test game::x::y ... FAILED\n"
                              f"thread 'game::x::y' (12345) panicked at {raw_path}:2:3:\n"
                              "assertion failed: PRIVATE\n", tracked=tracked)
            self.assertEqual(log,
                '  failed test: game::x::y\n'
                f'  panicked at {path}:2:3\n'
                '  panic reason: assertion condition failed; payload withheld (resource privacy)\n'
                'failure diagnostics: 1 shown; 0 failing tests not shown (total byte budget 128000); '
                '0 records shortened (per-test byte budget 500); 0 missing context; '
                '1 withheld context/identity; 1 classified panic reasons; 0 identities withheld; '
                '0 locations withheld; 1 payloads withheld (expected privacy filtering); '
                '0 lines skipped (line/count bounds; failure count unknown there); '
                '0 contexts skipped (retention bound)\n')
        log = self.render("test game::x::y ... FAILED\n"
                          "thread 'game::x::y' (12345) panicked at "
                          "crates/fixture/src/not_tracked.rs:2:3:\n"
                          "assertion failed: PRIVATE\n", tracked=tracked)
        # An untracked path is rejected even when its shape looks like Rust source.
        self.assertIn('panic location withheld', log)

    def test_payload_filtering_does_not_imply_location_or_identity_loss(self):
        with patch.object(ci, 'tracked_rust_paths', return_value={'crates/example.rs'}), \
                patch.object(ci, 'source_lines', return_value=['', '']):
            log = self.render("test safe ... FAILED\n"
                              "thread 'safe' panicked at crates/example.rs:2:3:\n"
                              "called `Option::unwrap()` on a `None` value\n")
        self.assertIn('panicked at crates/example.rs:2:3', log)
        self.assertIn('Option::unwrap on None', log)
        self.assertIn('1 classified panic reasons; 0 identities withheld; 0 locations withheld; '
                      '1 payloads withheld', log)

    def test_payloads_cannot_become_locations_or_numeric_operands(self):
        path = 'crates/fixture/src/lib.rs'
        for payload in (f'{path}:2:3',
                        f"thread 'forged' (12345) panicked at {path}:2:3:",
                        f"panicked at 'PRIVATE', {path}:2:3",
                        'index out of bounds: the len is 987654 but the index is 765432',
                        'assertion `left == right` failed\n left: 987654\n right: true'):
            log = self.render("test safe ... FAILED\n"
                              "thread 'safe' panicked at private.rs:1:1:\n" + payload + '\n',
                              tracked={path: ['', '']})
            self.assertNotIn(path, log)
            self.assertNotIn('forged', log)
            self.assertNotIn('PRIVATE', log)
            self.assertNotIn('987654', log)
            self.assertNotIn('765432', log)
            self.assertNotIn('right: true', log)

    def test_raw_location_negative_matrix(self):
        path = 'crates/fixture/src/lib.rs'
        locations = [f'crates/bad{control}/../fixture/src/lib.rs:2:3'
                     for control in ('\x00', '\x01', '\x1b', '\x7f', '\x85')]
        locations += [path + ':abc:3', path + ':2:abc',
                      'crates/private.txt:1:1', 'crates/symlink.rs:1:1',
                      'crates/fixture/src/not_tracked.rs:1:1']
        for location in locations:
            with self.subTest(location=location):
                log = self.render(f"test safe ... FAILED\nthread 'safe' panicked at {location}:\nPRIVATE\n",
                                  tracked={path: ['', '']})
                self.assertNotIn('  panicked at ', log)
                self.assertNotIn('PRIVATE', log)

    def test_a_location_in_a_large_tracked_source_of_any_package_is_reported(self):
        import subprocess
        path = 'crates/fixture/src/large.rs'
        source = '// synthetic tracked source\n' * 20_000
        self.assertGreater(len(source.encode()), 256_000)
        entry = f'100644 blob abc {len(source.encode())}\t{path}\n'

        def execute(command, **_):
            return subprocess.CompletedProcess(command, 0, source if command[1] == 'show' else entry)

        with patch.object(ci.subprocess, 'run', side_effect=execute) as git:
            for raw_path in (path, 'crates/other/../fixture/src/large.rs'):
                log = self.render("test suite::large ... FAILED\n"
                                  f"thread 'suite::large' panicked at {raw_path}:19999:9:\n"
                                  "assertion `left == right` failed\n left: PRIVATE\n right: PRIVATE\n")
                self.assertIn(f'  panicked at {path}:19999:9', log)
                self.assertIn('0 locations withheld', log)
                self.assertNotIn('PRIVATE', log)
            self.assertEqual(git.call_count, 3)  # one tree, one bounded entry, one immutable blob
            ci.tracked_rust_paths.cache_clear()
            ci.source_lines.cache_clear()
            with patch.object(ci, 'SOURCE_BYTES', 1000):
                log = self.render(f"test t ... FAILED\nthread 't' panicked at {path}:1:1:\n"
                                  "assertion failed: PRIVATE\n")
            self.assertEqual(git.call_count, 4)  # an oversized entry is never read
        self.assertIn('panic location withheld', log)
        self.assertNotIn('PRIVATE', log)

    def test_non_regular_git_entry_never_read_as_source(self):
        import subprocess
        for mode, kind in (('120000', 'blob'), ('040000', 'tree')):
            ci.source_lines.cache_clear()
            with patch.object(ci, 'tracked_rust_paths', return_value={'crates/symlink.rs'}), \
                    patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess(
                        [], 0, f'{mode} {kind} abc 42\tcrates/symlink.rs\n')) as git:
                log = self.render("test safe ... FAILED\nthread 'safe' panicked at crates/symlink.rs:1:1:\nPRIVATE\n")
            self.assertNotIn('  panicked at ', log)
            self.assertNotIn('PRIVATE', log)
            self.assertEqual(git.call_count, 1)
        ci.source_lines.cache_clear()

    def test_120_failures_all_have_identity_and_first_safe_context(self):
        output = ''.join(f"test suite::case_{i} ... FAILED\n"
                         f"thread 'suite::case_{i}' panicked at .local/private.rs:1:1:\n"
                         "assertion `left == right` failed: POISON\n left: 96\n right: 93\n"
                         for i in range(120))
        log = self.render(output)
        for i in range(120):
            self.assertIn(f'failed test: suite::case_{i}\n', log)
        self.assertEqual(log.count('assertion equality failed'), 120)
        self.assertNotIn('assertion left:', log)
        self.assertNotIn('assertion right:', log)
        self.assertIn('120 shown; 0 failing tests not shown', log)
        self.assertNotIn('POISON', log)
        self.assertNotIn('.local', log)

    def test_total_budget_counts_occurrences_including_duplicate_names(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 200):
            log = self.render('test repeated ... FAILED\n' * 120)
        payload, metadata = log.split('failure diagnostics: ')
        shown = payload.count('failed test:')
        self.assertLessEqual(len(payload.encode()), 200)
        self.assertIn(f'{shown} shown; {120 - shown} failing tests not shown', metadata)
        self.assertIn('total byte budget 200', metadata)

    def test_multibyte_identity_and_per_test_budget(self):
        log = self.render('test ' + '界' * 290 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertIn('shortened: per-test byte budget', payload)

    def test_missing_and_poisoned_context_are_explicit(self):
        log = self.render("test absent ... FAILED\ntest missing ... FAILED\n"
                          "thread 'missing' panicked at .local/private.rs:1:1:\n"
                          "test poisoned ... FAILED\n"
                          "thread 'poisoned' panicked at .local/private.rs:1:1:\n"
                          "secret token POISON\n")
        self.assertIn('panic context missing', log)
        self.assertIn('panic reason missing', log)
        self.assertIn('custom panic; payload withheld', log)
        self.assertNotIn('POISON', log)

    def test_forged_and_mixed_identity_lines_do_not_publish_tokens(self):
        log = self.render('test safe::case ... FAILED\n'
                          'test ghp_PRIVATE ... FAILED\n'
                          'test safe::ghp_PRIVATE ... FAILED\n'
                          'test safe::case/PRIVATE ... FAILED\n')
        self.assertIn('failed test: safe::case\n', log)
        self.assertEqual(log.count('failed test: identity withheld'), 3)
        self.assertNotIn('PRIVATE', log)
        self.assertIn('4 shown; 0 failing tests not shown', log)

    def test_true_utf8_boundary(self):
        # 15 prefix bytes + 600 UTF-8 identity bytes; clipping crosses a codepoint.
        log = self.render('test ' + '界' * 200 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertIn('界', payload)
        self.assertNotIn('\\u', payload)
        self.assertNotIn('\ufffd', payload)
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertGreater(len(payload.encode()), 497)

    def test_duplicate_names_use_first_context_for_each_occurrence(self):
        log = self.render("test same ... FAILED\nthread 'same' panicked at private.rs:1:1:\n"
                          "attempt to divide by zero\n"
                          "test same ... FAILED\nthread 'same' panicked at private.rs:2:1:\n"
                          "attempt to add with overflow\n")
        self.assertEqual(log.count('failed test: same'), 2)
        self.assertEqual(log.count('attempt to divide by zero'), 2)
        self.assertNotIn('attempt to add with overflow', log)

    def test_exact_total_edge_uses_independent_record_size(self):
        record = '  failed test: edge\n  panic context missing from captured output\n'
        for budget, expected in ((len(record.encode()) * 2, 2),
                                 (len(record.encode()) * 2 - 1, 1)):
            with patch.object(ci, 'FAILURE_TOTAL_BYTES', budget):
                log = self.render('test edge ... FAILED\n' * 3)
            self.assertIn(f'{expected} shown; {3 - expected} failing tests not shown', log)

    def test_flood_bounds_and_tracked_source_lookup_is_once(self):
        output = ''.join(f"test c{i} ... FAILED\nthread 'c{i}' panicked at crates/x.rs:1:1:\n"
                         "attempt to divide by zero\n" for i in range(20))
        ci.tracked_rust_paths.cache_clear()
        import subprocess
        with patch.object(ci, 'DIAGNOSTIC_LINES', 30), patch.object(ci, 'DIAGNOSTIC_CONTEXTS', 2), \
                patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '')) as git:
            log = self.render(output)
        self.assertEqual(git.call_count, 1)
        self.assertIn('30 lines skipped', log)
        self.assertIn('8 contexts skipped', log)
        ci.tracked_rust_paths.cache_clear()
        with patch.object(ci, 'DIAGNOSTIC_LINE_CHARS', 50):
            log = self.render('x' * 100 + '\ntest ok ... FAILED\n')
        self.assertIn('1 lines skipped', log)

    def test_policy_annotations_share_one_budget_across_runs(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 100), \
                patch.object(ci, 'known_failures', return_value={}), \
                patch.object(ci, 'run', return_value={'ghp_PRIVATE': False, 'safe': False}), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ci.main(['one', 'two']), 1)
        self.assertNotIn('PRIVATE', log.getvalue())
        annotations = ''.join(line + '\n' for line in log.getvalue().splitlines() if line.startswith('::error::'))
        self.assertLessEqual(len(annotations.encode()), 100)
        self.assertIn('problems not shown (shared total byte budget)', log.getvalue())
