import importlib.util
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent.parent / 'tools'
# Loaded by path: putting tools/ on sys.path would shadow same-named test modules in tests/ during discovery.
_spec = importlib.util.spec_from_file_location('api_scaffold', TOOLS / 'api_scaffold.py')
scaffold = importlib.util.module_from_spec(_spec)
sys.modules['api_scaffold'] = scaffold
_spec.loader.exec_module(scaffold)

CONFIG = textwrap.dedent('''
    [[type]]
    engine = "eng::Dir"
    kind = "enum"
    domain = "world"
    topic = "dir"
    doc = "A direction."
    variants = ["Up", "Down"]

    [[type]]
    engine = "eng::Pt"
    kind = "struct"
    domain = "world"
    topic = "point"
    doc = "A point."
    fields = ["x: i32", "y: i32"]

    [[type]]
    engine = "eng::Boat"
    kind = "handle"
    domain = "entity"
    topic = "boat"
    doc = "A boat."
    methods = [{ name = "is_wet", ret = "Option<bool>", doc = "Wet." }]
''')


class ScaffoldTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        for rel, text in {
            'crates/horizon-mod-vocab/src/lib.rs': 'pub struct Vec3;\n',
            'crates/horizon-mod-vocab/src/world/point.rs': '//! Hand-written.\npub struct Keep;\n',
            'crates/horizon-client/src/game/modding/api_values.rs':
                '#[path = "api_values/a.rs"]\npub(in crate::game) mod a;\n\nfn tail() {}\n',
        }.items():
            self.write(rel, text)
        self.config = self.root / 'cfg.toml'
        self.config.write_text(CONFIG)

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run_tool(self, *args):
        return subprocess.run([sys.executable, str(TOOLS / 'api_scaffold.py'), '--config', str(self.config),
                               '--root', str(self.root), *args], capture_output=True, text=True)

    def test_dry_run_writes_nothing(self):
        before = {p: p.read_text() for p in self.root.rglob('*.rs')}
        done = self.run_tool('--dry-run')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(before, {p: p.read_text() for p in self.root.rglob('*.rs')})
        self.assertEqual(len(list(self.root.rglob('*.rs'))), len(before))

    def test_existing_type_is_refused_and_nothing_is_written(self):
        self.write('crates/horizon-mod-vocab/src/other.rs', 'pub enum Dir {}\n')
        done = self.run_tool('--apply')
        self.assertNotEqual(done.returncode, 0)
        self.assertIn('already declared', done.stderr)
        self.assertFalse((self.root / 'crates/horizon-mod-vocab/src/exports/dir.rs').exists())

    def test_lib_is_never_a_target(self):
        self.config.write_text(CONFIG.replace('topic = "dir"', 'topic = "lib"'))
        done = self.run_tool('--apply')
        self.assertIn('lib.rs is never a target', done.stderr)
        self.assertEqual((self.root / 'crates/horizon-mod-vocab/src/lib.rs').read_text(), 'pub struct Vec3;\n')

    def test_apply_appends_and_creates(self):
        self.assertEqual(self.run_tool('--apply').returncode, 0)
        api = self.root / 'crates/horizon-mod-vocab/src'
        point = (api / 'world/point.rs').read_text()
        self.assertTrue(point.startswith('//! Hand-written.\npub struct Keep;\n'))
        self.assertIn('pub struct Pt {\n    /// The `x` component.\n    pub x: i32,', point)
        self.assertIn('pub const fn new(x: i32, y: i32) -> Self', point)
        self.assertEqual((api / 'exports/dir.rs').read_text().splitlines()[-1], 'pub use crate::world::dir::Dir;')
        self.assertIn('pub(crate) mod boat;', (api / 'entity/mod.rs').read_text())
        self.assertIn('horizon_mod_api_macros::Wire', point)
        self.assertIn('horizon_mod_api_macros::Wire', (api / 'world/dir.rs').read_text())
        self.assertIn('pub struct BoatRef {', (api / 'entity/boat.rs').read_text())
        self.assertIn('fn boat_is_wet(&self, handle: BoatRef) -> Option<bool>;', (api / 'host/boat.rs').read_text())
        conv = (self.root / 'crates/horizon-client/src/game/modding/api_values/dir.rs').read_text()
        self.assertIn('_ => return Err(UnsupportedDir)', conv)
        boat = (self.root / 'crates/horizon-client/src/game/modding/api_values/boat.rs').read_text()
        self.assertIn('pub(crate) fn boat_is_wet(&self, handle: mod_api::BoatRef) -> Option<bool>', boat)
        self.assertIn('horizon_mod_api_macros::Wire', (api / 'entity/boat.rs').read_text())
        values = (self.root / 'crates/horizon-client/src/game/modding/api_values.rs').read_text()
        self.assertIn('mod dir;', values)
        self.assertTrue(values.endswith('\n\nfn tail() {}\n'))
        self.assertLess(values.index('mod a;'), values.index('mod dir;'))

    def test_explicit_wire_derives_cover_all_emitted_values(self):
        self.assertEqual(self.run_tool('--apply').returncode, 0)
        api = self.root / 'crates/horizon-mod-vocab/src'
        for rel, declarations in [('world/dir.rs', ['Dir']), ('world/point.rs', ['Pt']),
                                  ('entity/boat.rs', ['BoatRef', 'BoatRefFields'])]:
            source = (api / rel).read_text()
            for name in declarations:
                self.assertRegex(source, rf'(?s)#\[derive\([^)]*horizon_mod_api_macros::Wire[^)]*\)\]'
                                 rf'(?:(?!pub (?:struct|enum)).)*pub (?:struct|enum) {name}\b')
            self.assertNotIn('impl crate::wire::Wire', source)
            self.assertNotIn('NativeWorldEpoch', source)
        self.assertIn('crate::WorldEpoch', (api / 'entity/boat.rs').read_text())

    def test_native_engine_names_are_preserved_only_on_engine_side(self):
        self.config.write_text(CONFIG.replace('eng::Pt', 'eng::NativePt'))
        self.assertEqual(self.run_tool('--apply').returncode, 0)
        source = (self.root / scaffold.API / 'world/point.rs').read_text()
        self.assertIn('pub struct Pt', source)
        conv = (self.root / scaffold.CONV / 'point.rs').read_text()
        self.assertIn('for eng::NativePt', conv)
        self.assertIn('for mod_api::Pt', conv)
        self.assertNotIn('mod_api::Native', conv)

    def test_shared_topic_is_registered_once(self):
        self.config.write_text(CONFIG.replace('topic = "point"', 'topic = "dir"'))
        self.assertEqual(self.run_tool('--apply').returncode, 0)
        api = self.root / scaffold.API
        self.assertEqual((api / 'world/mod.rs').read_text().count('mod dir;'), 1)
        source = (api / 'world/dir.rs').read_text()
        self.assertIn('pub enum Dir', source)
        self.assertIn('pub struct Pt', source)

    def test_exhaustive_struct_is_built_by_literal(self):
        self.config.write_text(CONFIG.replace('fields = ["x: i32", "y: i32"]',
                                              'fields = ["x: i32", "y: i32"]\nexhaustive = "two ints"'))
        self.assertEqual(self.run_tool('--apply').returncode, 0)
        conv = (self.root / 'crates/horizon-client/src/game/modding/api_values/point.rs').read_text()
        self.assertIn('mod_api::Pt { x: self.x, y: self.y }', conv)
        self.assertNotIn('pub const fn new', (self.root / 'crates/horizon-mod-vocab/src/world/point.rs').read_text())

    def test_handle_reads_must_be_optional(self):
        self.config.write_text(CONFIG.replace('ret = "Option<bool>"', 'ret = "bool"'))
        self.assertIn('returns Option', self.run_tool('--dry-run').stderr)

    def test_bad_names_are_refused_without_writing(self):
        for old, new in [('domain = "world"', 'domain = "../../escaped"'),
                         ('domain = "world"', 'domain = ["world"]'),
                         ('domain = "world"', ''),
                         ('topic = "dir"', 'topic = "mod"'),
                         ('topic = "dir"', 'topic = "dir"\nname = "NativeDir"'), ('topic = "dir"', 'topic = "../../escaped"'), ('topic = "dir"', 'topic = "host/../lib"'),
                         ('topic = "dir"', 'topic = "/tmp/abs"'), ('topic = "dir"', 'topic = "type"'),
                         ('variants = ["Up", "Down"]', 'variants = ["Up", "fn"]'),
                         ('fields = ["x: i32", "y: i32"]', 'fields = ["x: i32", "y z: i32"]'),
                         ('name = "is_wet"', 'name = "bad name"')]:
            with self.subTest(new=new):
                self.config.write_text(CONFIG.replace(old, new))
                before = sorted(p.relative_to(self.root) for p in self.root.rglob('*') if p.is_file())
                done = self.run_tool('--apply')
                self.assertNotEqual(done.returncode, 0)
                self.assertIn('refused', done.stderr)
                self.assertEqual(before, sorted(p.relative_to(self.root) for p in self.root.rglob('*') if p.is_file()))

    def test_second_apply_refuses(self):
        self.assertEqual(self.run_tool('--apply').returncode, 0)
        done = self.run_tool('--apply')
        self.assertNotEqual(done.returncode, 0)
        self.assertIn('already declared', done.stderr)


class RankTest(unittest.TestCase):
    def test_leaves_ignore_std_primitives_and_lifetimes(self):
        self.assertEqual(scaffold.leaves("Option < &'a glam :: DVec3 >"), {'DVec3'})
        self.assertEqual(scaffold.leaves('[i32 ; 3]'), set())
        self.assertEqual(scaffold.leaves("Game < '_ >"), set())

    def test_rank_counts_alone_and_touching(self):
        def point(target, args, ret='()'):
            return {'target': target, 'raw_shape': {
                'args': [{'ty': t} for t in args], 'return_ty': ret, 'receiver': {'ty': "Game < '_ >"}}}
        inventory = [point('a', ['Boat']), point('b', ['Boat', 'Map']), point('c', ['i32']),
                     point('a', ['Boat']), {'target': 'z', 'raw_shape': None}]
        blocked = scaffold.blockers(inventory, have=set())
        self.assertEqual(blocked, {'a': {'Boat'}, 'b': {'Boat', 'Map'}, 'c': set()})
        self.assertEqual(scaffold.rank(blocked), [('Boat', 1, 2), ('Map', 0, 1)])
        self.assertEqual(scaffold.rank(blocked, resolved={'Boat'}), [('Map', 1, 1)])

    def test_ref_and_key_names_carry_the_engine_type(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / 'crates/horizon-mod-vocab/src'
            src.mkdir(parents=True)
            (src / 'a.rs').write_text('pub struct MobRef;\npub use x::{Foo, Bar};\n')
            self.assertLessEqual({'MobRef', 'Mob', 'Foo', 'Bar'}, scaffold.available(Path(tmp)))


if __name__ == '__main__':
    unittest.main()
