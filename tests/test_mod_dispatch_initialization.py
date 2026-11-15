"""Conservative source guard for real platform dispatch in Rust tests.

All source modules in a crate share its lib/bin test process. Therefore require
initialization for each direct mod dispatch test, even without a current neighbour
that installs the platform. Generated fixture runtimes and vocabulary-only tests
cannot install the platform and are not consumers of this rule.
"""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TEST = re.compile(r'#\[test\]\s*(?:#\[[^\n]*\]\s*)*fn (\w+)\([^)]*\)\s*\{')
DISPATCH = re.compile(r'\bdispatch_(?!workgroups\b)\w+\s*\(|\bwith_dispatch\s*\(|__private::enter\s*\(|matrix_checks::baseline\s*\(')
# Qualified mod_platform:: helpers and imported helper names both match.
# These helpers initialize before invoking user code or publishing registrations.
READY = re.compile(r'\b(?:initialize|with_mods|install|install_parts|stick_part|install_counter|with_game|with_test_game)\s*\(')
ISOLATED = {
    'crates/horizon-mod-api/tests/late_instrumentation.rs',
    'crates/horizon-mod-api/tests/uninstalled_dispatch.rs',
}


def test_bodies(source):
    for match in TEST.finditer(source):
        depth, end = 1, match.end()
        while depth and end < len(source):
            depth += (source[end] == '{') - (source[end] == '}')
            end += 1
        yield match[1], source[match.end():end - 1]


def violations(root):
    failures = []
    for crate in ('horizon-mod-api', 'horizon-mod-wasm', 'horizon-mod-host',
                  'horizon-mod-points', 'horizon-mod-render', 'horizon-client',
                  'horizon-client-lib'):
        for path in (root / 'crates' / crate).rglob('*.rs'):
            relative = path.relative_to(root).as_posix()
            source = path.read_text()
            # Strip comments so explanatory mentions cannot satisfy initialization.
            source = re.sub(r'//[^\n]*|/\*.*?\*/', '', source, flags=re.S)
            bodies = list(test_bodies(source))
            if relative in ISOLATED:
                if len(bodies) != 1:
                    failures.append(relative + ': isolation requires exactly one test')
                continue
            for name, body in bodies:
                dispatch = DISPATCH.search(body)
                # This fixture constructs a Game through with_game_at first.
                game_fixture = relative == 'crates/horizon-client/src/game/blocks/cauldron_interactions/tests.rs' and 'fixture(' in body
                if dispatch and not game_fixture and not READY.search(body[:dispatch.start()]):
                    failures.append(f'{relative}:{name}: initialize before mod dispatch')
    return failures


class ModDispatchInitialization(unittest.TestCase):
    def test_real_dispatch_tests_initialize_first(self):
        self.assertEqual(violations(ROOT), [])

    def test_guard_accepts_platform_helpers(self):
        for helper in ("initialize", "with_mods", "install", "install_parts"):
            self.assertIsNotNone(READY.search(f"mod_platform::__private::{helper}(mods);"))

    def test_guard_rejects_uninitialized_dispatch(self):
        source = '#[test]\nfn poison() { mod_api::dispatch_client_tick_pre(&mut host); }'
        name, body = next(test_bodies(source))
        self.assertEqual(name, 'poison')
        self.assertIsNotNone(DISPATCH.search(body))
        self.assertIsNone(READY.search(body))


if __name__ == '__main__':
    unittest.main()
