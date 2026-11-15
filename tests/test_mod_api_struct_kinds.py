"""The mod API's public structs are built by mods or only received (arismoko/horizon#2862)."""
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from tools.mod_api_struct_kinds import census, defaults_and_constructors, structs

# Public structs a mod can build with a brace literal that have neither
# `Default` nor a constructor and are not `#[non_exhaustive]`. The number only
# is a ceiling: classification batches need not edit this shared line.
NEITHER_LIMIT = 141


def kinds(source, file=""):
    """Name -> (non_exhaustive, derives_default, literal) for a fixture."""
    return {name: rest for name, _, *rest in structs(source, file)}


class StructKindFixtures(unittest.TestCase):
    def test_a_struct_of_public_fields_is_built_by_literal(self):
        self.assertEqual(kinds('pub struct A { pub x: u8, pub y: Vec<(u8, u8)> }'),
                         {'A': [False, False, True]})

    def test_only_explicitly_hidden_platform_plumbing_is_excluded(self):
        marker = '// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.\n'
        source = (marker + '#[doc(hidden)]\npub struct EngineSocket { pub callback: fn() }\n\n'
                  '#[doc(hidden)]\npub struct HiddenModValue { pub x: u8 }\n\n'
                  + marker + 'pub struct MarkedModValue { pub x: u8 }\n\n'
                  'pub struct ModValue { pub x: u8 }\n')
        self.assertEqual(set(kinds(source, 'crates/horizon-mod-vocab/src/dispatch_scope.rs')),
                         {'HiddenModValue', 'MarkedModValue', 'ModValue'})

    def test_doc_hidden_alone_is_counted_even_inside_a_socket_file(self):
        source = '#[doc(hidden)]\npub struct ModValue { pub x: u8 }'
        self.assertEqual(kinds(source, 'crates/horizon-mod-vocab/src/dispatch_scope.rs'),
                         {'ModValue': [False, False, True]})

    def test_platform_marker_outside_socket_files_cannot_exempt_a_struct(self):
        source = ('// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.\n'
                  '#[doc(hidden)]\npub struct ModValue { pub x: u8 }')
        for file in ('crates/horizon-mod-vocab/src/item/types.rs',
                     'crates/horizon-mod-api/src/dispatch_scope.rs', ''):
            with self.subTest(file=file):
                self.assertEqual(kinds(source, file), {'ModValue': [False, False, True]})

    def test_platform_marker_does_not_leak_across_a_declaration(self):
        source = ('// hidden: platform-owned instrumentation plumbing, not a mod authoring surface.\n'
                  '#[doc(hidden)]\npub struct EngineSocket { pub callback: fn() }\n'
                  '#[doc(hidden)]\npub struct HiddenModValue { pub x: u8 }')
        self.assertEqual(set(kinds(source, 'crates/horizon-mod-vocab/src/dispatch_scope.rs')),
                         {'HiddenModValue'})

    def test_a_private_or_crate_field_stops_a_literal(self):
        found = kinds('pub struct A { pub x: u8, y: u8 } pub struct B { pub(crate) x: u8 }')
        self.assertEqual(found['A'][2], False)
        self.assertEqual(found['B'][2], False)

    def test_tuple_and_unit_structs_are_not_counted_as_literals(self):
        found = kinds('pub struct A(pub u8); pub struct B;')
        self.assertEqual([found['A'][2], found['B'][2]], [False, False])

    def test_attributes_say_non_exhaustive_and_default(self):
        found = kinds('#[non_exhaustive]\n#[derive(Clone)] pub struct A { pub x: u8 }\n'
                      '#[derive(Clone, Default, Wire)] pub struct B { pub x: u8 }\n'
                      '#[derive(Clone)] pub struct C { #[doc = "x"] pub x: u8 }')
        self.assertEqual(found['A'][:2], [True, False])
        self.assertEqual(found['B'][:2], [False, True])
        self.assertEqual(found['C'], [False, False, True])

    def test_generic_and_lifetime_structs_are_read(self):
        found = kinds("pub struct A<'a, T: Into<u8>> where T: Clone { pub x: &'a T }")
        self.assertEqual(found['A'], [False, False, True])

    def test_built_mirrors_are_left_out(self):
        self.assertEqual(kinds('mod __private { pub(crate) mod built { pub struct A { pub x: u8 } } }'), {})

    def test_test_only_structs_are_left_out(self):
        self.assertEqual(kinds('#[cfg(test)] mod tests { pub struct A { pub x: u8 } } }'), {})

    def test_default_impls_and_constructors_are_found(self):
        defaults, constructors = defaults_and_constructors(
            'impl Default for A { fn default() -> Self { A { x: 0 } } }\n'
            'impl B { pub fn new(x: u8) -> Self { B { x } } pub fn x(&self) -> u8 { self.x } }\n'
            'impl C { pub fn x(&self) -> Self { todo!() } }\n'
            'impl D { pub fn parse(s: &str) -> Option<Self> { None } }')
        self.assertEqual(defaults, {('', ('A',))})
        self.assertEqual(constructors, {('', ('B',)), ('', ('D',))})

    def test_arrow_where_and_unrelated_built(self):
        self.assertFalse(kinds('pub struct A { pub cb: fn() -> u8, private: u8 }')['A'][2])
        self.assertTrue(kinds('pub struct A<T> where T: Fn() { pub x: T }')['A'][2])
        self.assertIn('A', kinds('mod built { pub struct A { pub x: u8 } }'))

    def test_scoped_exact_constructors(self):
        d, c = defaults_and_constructors(
            'mod a { impl Default for A {} impl A { pub fn new() -> Self { todo!() } } } '
            'mod b { impl A { pub fn wrong() -> Self::Other { todo!() } } } '
            'impl C { pub fn new() -> Option<C> { todo!() } } '
            'impl D { pub fn new() -> Result<D, E> { todo!() } }', 'file')
        self.assertEqual(d, {('file', ('a', 'A'))})
        self.assertEqual(c, {('file', ('a', 'A')), ('file', ('C',)), ('file', ('D',))})

    def test_file_backed_module_prefixes_exclude_only_mirrors(self):
        with TemporaryDirectory() as folder:
            root = Path(folder)
            src = root / 'crates/horizon-mod-vocab/src'
            fixtures = {
                '__private.rs': 'mod built { pub struct Hidden { pub x: u8 } }',
                '__private/built/mod.rs': 'pub struct AlsoHidden { pub x: u8 }',
                '__private/built/leaf.rs': 'pub struct HiddenLeaf { pub x: u8 }',
                'ordinary/built.rs': 'pub struct Visible { pub x: u8 }',
                'ordinary/mod.rs': 'pub struct Defaulted { pub x: u8 } '
                                   'impl Default for Defaulted {}',
                'ordinary/leaf.rs': 'mod nested { pub struct Constructed { pub x: u8 } '
                                    'impl Constructed { pub fn new() -> Self { todo!() } } }',
            }
            for file, source in fixtures.items():
                path = src / file
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)
            counts, neither = census(root)
            self.assertEqual(counts['public'], 3)
            self.assertEqual(counts['default'], 1)
            self.assertEqual(counts['constructor'], 1)
            self.assertEqual(counts['neither'], 1)
            self.assertEqual(neither, [
                'crates/horizon-mod-vocab/src/ordinary/built.rs:1: Visible'])

    def test_file_prefix_is_part_of_impl_keys(self):
        for file, module in [
            ('crates/horizon-mod-vocab/src/foo.rs', ('foo',)),
            ('crates/horizon-mod-vocab/src/foo/mod.rs', ('foo',)),
            ('crates/horizon-mod-vocab/src/__private.rs', ('__private',)),
            ('crates/horizon-mod-vocab/src/lib.rs', ()),
        ]:
            with self.subTest(file=file):
                defaults, constructors = defaults_and_constructors(
                    'mod inner { impl Default for A {} '
                    'impl B { pub fn new() -> Self { todo!() } } }', file)
                self.assertEqual(defaults, {(file, module + ('inner', 'A'))})
                self.assertEqual(constructors, {(file, module + ('inner', 'B'))})

    def test_same_line_declarations_keep_distinct_module_keys(self):
        with TemporaryDirectory() as folder:
            root = Path(folder)
            path = root / 'crates/horizon-mod-vocab/src/foo.rs'
            path.parent.mkdir(parents=True)
            path.write_text(
                'mod a { pub struct A { pub x: u8 } impl Default for A {} } '
                'mod b { pub struct A { pub x: u8 } }')
            counts, neither = census(root)
            self.assertEqual(counts['public'], 2)
            self.assertEqual(counts['default'], 1)
            self.assertEqual(counts['neither'], 1)
            self.assertEqual(neither, ['crates/horizon-mod-vocab/src/foo.rs:1: A'])


class StructKindRepository(unittest.TestCase):
    def test_no_new_struct_is_neither_kind(self):
        counts, neither = census()
        listed = '\n'.join(neither)
        self.assertLessEqual(
            counts['neither'], NEITHER_LIMIT,
            'a public mod API struct is built by literal but has neither Default nor a '
            'constructor: add Default (kind 1, built by mods) or #[non_exhaustive] (kind 2, '
            f'only received; see docs/modding). Neither kind now:\n{listed}')


if __name__ == '__main__':
    unittest.main()
