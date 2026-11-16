"""Fixture checks for the source-only Wire boundary policy."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from tools.check_mod_api_wire import (
    API_TEMPLATE_FILES, violations, template_violations,
    repository_violations, runtime_reexport_names,
)


class WireGuardFixtures(unittest.TestCase):
    def test_cli_names_the_type_and_explains_both_fixes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / 'tools/check_mod_api_wire.py'
            script.parent.mkdir()
            script.write_text((Path(__file__).resolve().parents[1] / 'tools/check_mod_api_wire.py').read_text())
            api = root / 'crates/horizon-mod-api/src/lib.rs'
            api.parent.mkdir(parents=True)
            api.write_text('pub struct MissingType;')
            for relative in API_TEMPLATE_FILES:
                template = root / 'crates/horizon-mod-api-gen/src' / relative
                template.parent.mkdir(parents=True, exist_ok=True)
                template.write_text('')
            refused = subprocess.run([sys.executable, str(script)], capture_output=True, text=True)
            self.assertEqual(refused.returncode, 1)
            self.assertIn('MissingType', refused.stdout)
            self.assertIn('#[derive(mod_api::Wire)]', refused.stdout)
            self.assertIn('// no wire: <reason>', refused.stdout)
            api.write_text('#[derive(mod_api::Wire)] pub struct MissingType;')
            accepted = subprocess.run([sys.executable, str(script)], capture_output=True, text=True)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)
            vocab = root / 'crates/horizon-mod-vocab/src/lib.rs'
            vocab.parent.mkdir(parents=True)
            vocab.write_text('pub struct MissingVocabularyForm;')
            refused_vocab = subprocess.run([sys.executable, str(script)], capture_output=True, text=True)
            self.assertEqual(refused_vocab.returncode, 1)
            self.assertIn('MissingVocabularyForm', refused_vocab.stdout)
            vocab.write_text('#[derive(mod_api::Wire)] pub struct MissingVocabularyForm;')
            self.assertEqual(repository_violations(root), [])
            api.write_text('mod checked; mod collision;')
            (api.parent / 'checked.rs').write_text(
                '// no wire: checked manual codec\npub struct Shared;\nimpl Wire for Shared {}')
            (api.parent / 'collision.rs').write_text('pub struct Shared;')
            collision = subprocess.run([sys.executable, str(script)], capture_output=True, text=True)
            self.assertEqual(collision.returncode, 1)
            self.assertIn('collision.rs:1: Shared', collision.stdout)
            self.assertNotIn('checked.rs:', collision.stdout)

    def test_runtime_reexports_check_the_definition_not_the_alias_or_manual_impl(self):
        source = '''pub use {other::Ignored, horizon_mod_runtime::attachments::{
            AttachmentError as PublicError, nested::{AttachmentRowError},
        }};
        #[cfg(test)] mod tests { pub use horizon_mod_runtime::OnlyTest; }
        '''
        self.assertEqual(runtime_reexport_names(source),
                         {'AttachmentError', 'AttachmentRowError'})
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            api = root / 'crates/horizon-mod-api/src/lib.rs'
            api.parent.mkdir(parents=True)
            api.write_text(source)
            runtime = root / 'crates/horizon-mod-runtime/src/attachments.rs'
            runtime.parent.mkdir(parents=True)
            runtime.write_text('pub enum AttachmentError {}\npub struct AttachmentRowError;\n'
                               'pub struct Unexported;\nimpl Wire for AttachmentError {}')
            for relative in API_TEMPLATE_FILES:
                template = root / 'crates/horizon-mod-api-gen/src' / relative
                template.parent.mkdir(parents=True, exist_ok=True)
                template.write_text('')
            failures = repository_violations(root)
            self.assertEqual(len(failures), 2)
            self.assertTrue(any('AttachmentError' in f for f in failures))
            self.assertTrue(any('AttachmentRowError' in f for f in failures))
            runtime.write_text('// no wire: runtime-owned explicit codec\n'
                               'pub enum AttachmentError {}\n'
                               '#[derive(Wire)] pub struct AttachmentRowError;\n'
                               'pub struct Unexported;')
            self.assertEqual(repository_violations(root), [])
            api.write_text('pub use horizon_mod_runtime::attachments::*;')
            self.assertTrue(any('wildcard' in f for f in repository_violations(root)))

    def test_missing_declarations_include_aliases_and_generics(self):
        source = 'pub struct A<T>(T); pub enum B { A } pub type C<T> = Vec<T>;'
        self.assertEqual([n for _, n in violations(source)], ['A', 'B', 'C'])

    def test_explicit_derive_and_attached_reason(self):
        source = '''#[derive(Clone, mod_api::Wire)]
        pub struct A<T>(T);
        // no wire: a host-only callback
        #[derive(Clone)]
        pub type Callback = fn();
        #[derive(
            Debug,
            Wire,
        )]
        pub enum B { A }
        '''
        self.assertEqual(violations(source), [])

    def test_reason_does_not_exempt_following_items_or_blank_reason(self):
        source = '''// no wire: host service
pub struct A;
pub struct B;
// no wire:
pub enum C {}
'''
        self.assertEqual([n for _, n in violations(source)], ['B', 'C'])

    def test_handwritten_generic_impl_and_macro_families_require_attached_policy(self):
        source = '''pub struct A<T>(T); pub struct Key; pub struct Handle;
impl<T: Wire> crate::wire::Wire for A<T> {}
macro_rules! slot_keys { ($repr:ty; $($key:ty),*) => { $(impl Wire for $key {})* }; }
slot_keys!(u32; crate::Key);
macro_rules! entity_handles { ($($ty:ty),*) => { $(impl Wire for $ty {})* }; }
entity_handles!(Handle);
'''
        self.assertEqual([n for _, n in violations(source)], ['A', 'Key', 'Handle'])

    def test_macro_name_alone_is_not_an_exemption(self):
        source = 'pub struct Key; macro_rules! slot_keys { () => {} } slot_keys!(Key);'
        self.assertEqual([n for _, n in violations(source)], ['Key'])

    def test_comments_strings_private_and_test_examples_are_not_api(self):
        source = '''/// pub struct DocExample;
/* outer /* nested */ pub enum Comment {} */
const EXAMPLE: &str = r##"pub struct Quoted;"##;
#[cfg(test)] mod tests { pub struct TestOnly; mod nested { pub enum Test {} } }
pub(crate) struct Private;
struct Local;
pub struct Real;
'''
        self.assertEqual([n for _, n in violations(source)], ['Real'])

    def test_conditional_test_modules_are_not_production(self):
        source = '''#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests { #[derive(Debug)] pub struct LootDrops; }
#[cfg(all(feature = "foo", any(test, all(test, feature = "bar"))))]
mod more_tests { pub enum OnlyTest {} }
#[cfg(any(test, feature = "foo"))] pub struct Production;
#[cfg(not(test))] pub struct AlsoProduction;
'''
        self.assertEqual([n for _, n in violations(source)], ['Production', 'AlsoProduction'])

    def test_reason_between_attributes_and_before_attributes(self):
        for source in (
            '// no wire: internal carrier\n#[doc(hidden)]\n#[allow(missing_docs)]\npub struct Carrier;',
            '#[doc(hidden)]\n#[allow(missing_docs)]\n// no wire: internal carrier\npub struct Carrier;',
        ):
            self.assertEqual(violations(source), [])
        self.assertEqual([n for _, n in violations('// no wire:\npub struct Missing;')], ['Missing'])

    def test_literal_policy_fragments_require_every_branch(self):
        source = '''fn emit() {
let wire_form = if generics.is_empty() {
    "#[derive(horizon_mod_api_macros::Wire)]"
} else {
    "// no wire: borrowed event crosses as call arguments."
};
format!("#[allow(missing_docs)]\\n{wire_form}\\npub struct {event_type}{generics} {{\\n{fields}}}\\n");
format!("pub struct {new_type} {{}} ");
}'''
        self.assertEqual([n for _, _, n in template_violations(source)], ['{new_type'])
        bad = source.replace('// no wire: borrowed event crosses as call arguments.', '')
        self.assertEqual([n for _, _, n in template_violations(bad)], ['{event_type', '{new_type'])
        reassigned = source.replace('format!("#[allow', 'let wire_form = ""; format!("#[allow', 1)
        self.assertEqual([n for _, _, n in template_violations(reassigned)], ['{event_type', '{new_type'])
        out_of_scope = source.replace('format!("#[allow', '} fn other() { format!("#[allow', 1)
        self.assertEqual([n for _, _, n in template_violations(out_of_scope)], ['{event_type', '{new_type'])

    def test_generated_templates_and_macro_declarations_are_checked(self):
        self.assertEqual([n for _, n in violations('pub struct {carrier}{generics} {{ {fields} }}')], ['{carrier'])
        self.assertEqual(violations('#[derive(Wire)] pub struct {carrier}{generics} {{ {fields} }}'), [])
        self.assertEqual([n for _, n in violations('pub struct $name;')], ['$name'])


if __name__ == '__main__':
    unittest.main()
