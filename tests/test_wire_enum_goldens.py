import importlib.util
import json
from pathlib import Path
import unittest
import sys

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("wire_goldens", ROOT / "tools/generate_wire_enum_goldens.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
sys.modules["generate_wire_enum_goldens"] = MODULE
CAPTURE_SPEC = importlib.util.spec_from_file_location("wire_capture", ROOT / "tools/capture_wire_enum_layouts.py")
CAPTURE = importlib.util.module_from_spec(CAPTURE_SPEC)
CAPTURE_SPEC.loader.exec_module(CAPTURE)


class WireEnumDiscoveryTests(unittest.TestCase):
    def test_payloads_comments_attributes_and_non_wire_enums(self):
        source = '''
        enum Plain { /// ignored punctuation (not a payload)
            #[doc = "{ fields } (tuple)"] First = 7, Second = 9 }
        enum Payload { Unit, Tuple(u32), Record { value: String } }
        enum Other { First }
        #[automatically_derived] impl ::mod_api::wire::Wire for Plain { fn layout() { runtime_before_or_const_after() } }
        #[automatically_derived] impl ::mod_api::wire::Wire for Payload {}
        '''
        self.assertEqual(MODULE.discover(source), {"crate::Plain"})

    def test_quotes_and_braces_inside_literals_are_not_syntax(self):
        source = r'''
        mod names {
            fn portable(c: char) -> bool { matches!(c, '/' | '\\' | '"' | '{' | '\u{7f}') }
            fn message() -> &'static str { "a mod's state crosses one link \
                group: not { every target" }
            #[doc = r#" a raw "quote and } brace"#] fn raw() {}
            enum Plain { A, B }
            #[automatically_derived] impl ::mod_api::wire::Wire for Plain {}
        }
        '''
        self.assertEqual(MODULE.discover(source), {"crate::names::Plain"})

    def test_unsupported_local_fieldless_impl_headers_fail_closed(self):
        for header in [
            "impl<const N: usize> ::mod_api::wire::Wire for Flag<N>",
            "impl ::mod_api::wire::Wire for crate::Flag",
            "impl ::mod_api::wire::Wire for Flag where (): Sized",
            "impl Wire for Flag",
        ]:
            declaration = "enum Flag<const N: usize> { A }" if "const N" in header else "enum Flag { A }"
            for attributes in [
                "#[automatically_derived] #[allow(unused)]",
                "#[allow(unused)] #[automatically_derived]",
            ]:
                with self.subTest(header=header, attributes=attributes):
                    with self.assertRaisesRegex(ValueError, "unsupported.*Flag"):
                        MODULE.discover(f"{declaration} {attributes} {header} {{}}")

    def test_supported_derive_with_both_attribute_orders(self):
        for attributes in [
            "#[automatically_derived] #[allow(unused)]",
            "#[allow(unused)] #[automatically_derived]",
        ]:
            self.assertEqual(MODULE.discover(f'''
                enum Plain {{ A, B }}
                {attributes} impl ::mod_api::wire::Wire for Plain {{}}
            '''), {"crate::Plain"})

    def test_manual_codecs_do_not_enter_derive_inventory(self):
        self.assertEqual(MODULE.discover('''
            enum Local { A }
            enum Manual { A }
            #[automatically_derived] impl ::mod_api::wire::Wire for Local {}
            impl crate::wire::Wire for Manual {}
            impl crate::wire::Wire for external::Other {}
        '''), {"crate::Local"})

    def test_capture_requires_complete_symbol_identity_and_reads_recorded_flags(self):
        symbols = CAPTURE.symbol_map(["0000 T original_symbol"],
            ["0000 T <mod_vocab::old::Same as mod_vocab::wire::Wire>::layout"], {"crate::old::Same"})
        self.assertEqual(symbols, {"crate::old::Same": "original_symbol"})
        with self.assertRaisesRegex(ValueError, "missing layout symbols"):
            CAPTURE.symbol_map([], [], {"crate::old::Same"})
        library, directory, externs = CAPTURE.build_inputs(
            "Running `rustc --crate-name mod_vocab -C extra-filename=-abc --out-dir target/debug/deps "
            "--extern core=target/debug/deps/libcore-abc.rmeta --extern macros=target/debug/deps/macros.so`")
        self.assertEqual(library, Path("target/debug/deps/libmod_vocab-abc.rlib"))
        self.assertEqual(directory, Path("target/debug/deps"))
        self.assertEqual(externs, ["core=target/debug/deps/libcore-abc.rlib"])

    def test_hook_id_guard_uses_same_source_reference_not_captured_constant(self):
        manifest = json.loads(MODULE.GOLDENS.read_text())
        hook = next(entry for entry in manifest["enums"] if entry["canonical_path"] == "crate::generated::HookId")
        before = MODULE.render({"enums": [hook]})
        self.assertIn("super::hook_id_runtime_reference()", before)
        self.assertNotIn(hook["baseline_hash"], before)
        hook["baseline_hash"] = "0000000000000000"
        self.assertEqual(MODULE.render({"enums": [hook]}), before)

    def test_deleting_a_real_frozen_pin_is_rejected_before_discovery(self):
        manifest = json.loads(MODULE.GOLDENS.read_text())
        required = json.loads((ROOT / "tools/fixtures/wire_enum_baseline_identities.json").read_text())
        del manifest["enums"][0]
        with self.assertRaisesRegex(ValueError, "missing.*pin"):
            MODULE.validate(manifest, required, "")

    def test_removed_or_non_wire_enum_requires_paired_pin_removal(self):
        identity = "crate::Gone"
        manifest = {"enums": [{"canonical_path": identity}]}
        for source in ["", "enum Gone { A }"]:
            with self.subTest(source=source):
                with self.assertRaisesRegex(ValueError, "Remove its matching pins.*SAME PR"):
                    MODULE.validate(manifest, [identity], source)
                retired = {"enums": [], "retired_identities": [identity]}
                self.assertEqual(MODULE.validate(retired, [], source), set())
                with self.assertRaisesRegex(ValueError, "missing.*pin"):
                    MODULE.validate(retired, [identity], source)

    def test_paired_pin_removal_cannot_retire_live_wire_enum(self):
        source = "enum Live { A } #[automatically_derived] impl ::mod_api::wire::Wire for Live {}"
        with self.assertRaisesRegex(ValueError, "cannot retire a live.*crate::Live"):
            MODULE.validate({"enums": [], "retired_identities": ["crate::Live"]}, [], source)

    def test_frozen_pins_allow_new_enum_and_leaf_collision(self):
        source = """
            mod old { enum Same { A } #[automatically_derived] impl ::mod_api::wire::Wire for Same {} }
            mod new { enum Same { B } #[automatically_derived] impl ::mod_api::wire::Wire for Same {} }
            enum Added { A } #[automatically_derived] impl ::mod_api::wire::Wire for Added {}
        """
        manifest = {"enums": [{"canonical_path": "crate::old::Same"}]}
        self.assertEqual(MODULE.validate(manifest, ["crate::old::Same"], source), {"crate::old::Same"})
        with self.assertRaisesRegex(ValueError, "missing.*pin"):
            MODULE.validate({"enums": []}, ["crate::old::Same"], source)
        with self.assertRaisesRegex(ValueError, "Remove its matching pins.*SAME PR"):
            MODULE.validate(manifest, ["crate::old::Same"], source.replace("mod old", "mod removed"))


if __name__ == "__main__":
    unittest.main()
