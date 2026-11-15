"""Executable negative controls for the API boundary source audit."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.ci import check_api_boundary as boundary


class ApiBoundaryCheckTests(unittest.TestCase):
    def findings(self, source, path="src/lib.rs"):
        return boundary.source_findings(Path(path), source)

    def assert_rule(self, source, rule="engine-type", path="src/lib.rs"):
        found = self.findings(source, path)
        self.assertIn(rule, {f.rule for f in found}, found)

    def test_negative_control(self):
        self.assertTrue(boundary.negative_control())

    def test_negative_control_kills_disabled_rules(self):
        real_check = boundary.check
        for rule in ("engine-type", "engine-dependency", "json-signature", "private-export"):
            with self.subTest(rule=rule):
                def mutant(crate):
                    return [f for f in real_check(crate) if f.rule != rule]
                with patch.object(boundary, "check", mutant):
                    self.assertFalse(boundary.negative_control())

    def test_private_type_alias_is_not_a_loophole(self):
        self.assert_rule("type X = horizon_world::fluid::FluidState; pub fn f() -> X {}")

    def test_aliased_export(self):
        self.assert_rule("pub use horizon_items::items::ItemStack as Stack;")

    def test_nested_aliased_export(self):
        self.assert_rule("pub use horizon_world::{fluid::{FluidState as Fluid}};")

    def test_glob_export(self):
        self.assert_rule("pub use horizon_client_core::text::*;")

    def test_private_import_alias_in_public_signature(self):
        self.assert_rule("use horizon_world::fluid::FluidState as X; pub fn f() -> X { todo!() }")

    def test_module_alias_in_public_signature(self):
        self.assert_rule("use horizon_world as engine; pub fn f(x: engine::fluid::FluidState) {}")

    def test_private_alias_chain(self):
        self.assert_rule("use horizon_world as engine; use engine::fluid::FluidState as X; pub fn f(x: X) {}")

    def test_child_scope_cannot_mask_an_engine_import(self):
        self.assert_rule('''use horizon_core::density::DensityFunction as X;
            #[cfg(test)] mod tests { use crate::X; }
            pub fn leak() -> X {}''')
        self.assert_rule('''mod child { use horizon_world as engine;
            pub fn leak() -> engine::fluid::FluidState {} }''')

    def test_fields_and_enum_payloads(self):
        self.assert_rule("pub struct S { pub x: horizon_items::items::ItemStack }")
        self.assert_rule("pub enum E { X(horizon_items::items::ItemStack) }")

    def test_tuple_struct_and_type_alias(self):
        self.assert_rule("pub struct S(pub horizon_items::items::ItemStack);")
        self.assert_rule("pub type S = horizon_items::items::ItemStack;")

    def test_trait_method_not_explicitly_pub(self):
        self.assert_rule("pub trait T { fn f(&self) -> horizon_world::fluid::FluidState; }")

    def test_export_fragments_and_nonprivate_hidden(self):
        self.assert_rule("#[doc(hidden)] pub use horizon_world::fluid::FluidState;", path="src/exports/area.rs")
        self.assert_rule("pub mod game { pub use horizon_world::fluid::*; }", path="src/exports/area.rs")

    def test_private_scope_is_the_only_escape_hatch(self):
        self.assertEqual(self.findings("pub mod __private { pub use horizon_world::fluid::*; }"), [])
        self.assertEqual(self.findings("pub use horizon_world::fluid::*;", "src/__private.rs"), [])
        self.assert_rule("pub mod __private { pub use horizon_world::fluid::*; } pub use horizon_items::*;")

    def test_private_reexports_cannot_be_laundered_through_aliases(self):
        self.assert_rule('pub use crate::__private::FluidState as Fluid;', "private-export")
        self.assert_rule('use crate::__private as hidden; pub use hidden::FluidState as Fluid;', "private-export")
        self.assert_rule('pub use horizon_world::*;', path="src/exports/__private.rs")

    def test_json_signature_and_alias(self):
        self.assert_rule("pub fn f() -> serde_json::Value {}", "json-signature")
        self.assert_rule("use serde_json::Value as V; pub struct S { pub data: V }", "json-signature")

    def test_json_exceptions_are_narrow(self):
        self.assertEqual(self.findings("pub fn from_json(v: &serde_json::Value) {}", "src/fluid_capability.rs"), [])
        self.assert_rule("pub fn other(v: &serde_json::Value) {}", "json-signature", "src/fluid_capability.rs")
        self.assert_rule("pub fn from_json(v: &serde_json::Value) {}", "json-signature")
        self.assertEqual(self.findings("pub use serde_json;", "src/exports/serde_json.rs"), [])

    def test_recipe_source_file_exception_names_only_field_and_constructor(self):
        path = "src/crafting.rs"
        self.assertEqual(self.findings('pub struct Recipe { pub json: std::sync::Arc<serde_json::Value> }', path), [])
        self.assertEqual(self.findings('impl Recipe { pub fn new(json: serde_json::Value) -> Self {} }', path), [])
        self.assert_rule('pub struct Recipe { pub metadata: serde_json::Value }', "json-signature", path)
        self.assert_rule('impl Recipe { pub fn other() -> serde_json::Value {} }', "json-signature", path)
        self.assert_rule('pub struct Other { pub json: serde_json::Value }', "json-signature", path)

    def test_core_allowlist_is_leaf_exact(self):
        self.assertEqual(self.findings("pub use horizon_core::nbt::{Tag, Compound};"), [])
        self.assert_rule("pub use horizon_core::*;")
        self.assert_rule("pub use horizon_core::nbt::*;")
        self.assert_rule("pub use horizon_core::density::DensityFunction;")

    def test_strings_comments_and_implementation_are_not_signatures(self):
        source = '''
        // pub use horizon_world::fluid::*;
        /* nested /* comment */ pub use horizon_world::*; */
        pub fn f() { let _ = horizon_world::fluid::FluidState::default(); }
        const TEXT: &str = r###"pub use horizon_world::*;"###;
        '''
        self.assertEqual(self.findings(source), [])

    def test_test_only_module_is_not_surface(self):
        self.assertEqual(self.findings("#[cfg(test)] mod tests { pub use horizon_world::*; }"), [])
        self.assert_rule("#[cfg(feature = \"bad\")] pub mod x { pub use horizon_world::*; }")

    def test_generated_placeholder_types_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            self.assertEqual(boundary.generated_findings(target)[0].rule, "missing-generated")
            out = target / "debug/build/horizon-mod-vocab-abcdef/out"
            out.mkdir(parents=True)
            (out / "mod_api_generated.rs").write_text('pub struct BlocksEvent { pub blocks: Vec<horizon_mod_package::content::Block> }')
            (out / "mod_api_generated_host.rs").write_text('pub trait Host { fn leak() -> serde_json::Value; }')
            found = boundary.generated_findings(target)
            self.assertEqual({f.rule for f in found}, {"engine-type", "json-signature"})
            (out / "mod_api_generated.rs").write_text('pub struct BlocksEvent { pub blocks: Vec<crate::BlockDeclaration> }')
            (out / "mod_api_generated_host.rs").write_text('pub trait Host {}')
            fixture = target / "debug/build/horizon-mod-api-gen-tests-fedcba/out"
            fixture.mkdir(parents=True)
            (fixture / "mod_api_generated.rs").write_text('pub use horizon_world::*;')
            self.assertEqual(boundary.generated_findings(target), [])
            (out / "mod_api_generated_host.rs").unlink()
            missing = boundary.generated_findings(target)
            self.assertEqual([(f.rule, f.detail) for f in missing],
                             [("missing-generated", "mod_api_generated_host.rs")])

    def test_dependency_aliases_and_target_tables(self):
        with tempfile.TemporaryDirectory() as directory:
            crate = Path(directory)
            (crate / "src").mkdir()
            (crate / "Cargo.toml").write_text('''
            [dependencies]
            horizon-core = "1"
            [target.'cfg(unix)'.dependencies]
            innocent = { package = "horizon-world", version = "1" }
            [dev-dependencies]
            horizon-items = "1"
            ''')
            found = boundary.check(crate)
            self.assertEqual({f.detail for f in found}, {"horizon-world", "horizon-items"})
            self.assertTrue(all(f.rule == "engine-dependency" for f in found))


if __name__ == "__main__":
    unittest.main()
