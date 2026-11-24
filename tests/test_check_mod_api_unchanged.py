import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import check_mod_api_unchanged as gate


NOTES = "# Hook notes\n\nGeneral.\n\n## Existing\n\nOld.\n\n```text\n## not a heading\n```\n"

class Snapshots:
    """Two snapshot folders in a temporary folder, filled from file maps."""

    def __init__(self, test):
        self.tmp = tempfile.TemporaryDirectory()
        test.addCleanup(self.tmp.cleanup)
        self.base, self.head = Path(self.tmp.name, "base"), Path(self.tmp.name, "head")

    def write(self, folder, files=None):
        folder.mkdir(parents=True, exist_ok=True)
        for name, text in (files or {}).items():
            (folder / name).parent.mkdir(parents=True, exist_ok=True)
            (folder / name).write_text(text)

    def run(self, base_files=None, head_files=None, **kwargs):
        self.write(self.base, base_files)
        self.write(self.head, head_files)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = gate.compare(self.base, self.head, **kwargs)
        return status, out.getvalue()


class VerdictTests(unittest.TestCase):
    def verdict(self, files_added=(), files_removed=(), diffs=(), explained=False, allow_added=False, internals=None,
                failing=(), stale=()):
        return gate.verdict(list(files_added), list(files_removed), list(diffs), explained, allow_added, internals,
                            list(failing), list(stale))

    def test_no_difference_passes(self):
        self.assertEqual(self.verdict(), 0)

    def test_a_generated_diff_needs_an_added_hook(self):
        self.assertEqual(self.verdict(diffs=[("lib.rs", ["-x"])]), 1)
        self.assertEqual(self.verdict(diffs=[("lib.rs", ["-x"])], explained=True), 0)

    def test_an_unlisted_removal_or_a_stale_listing_always_fails(self):
        self.assertEqual(self.verdict(failing=[("a", "fn", "")], explained=True, allow_added=True), 1)
        self.assertEqual(self.verdict(stale=["a"], explained=True, allow_added=True), 1)

    def test_a_strict_file_the_head_dropped_fails_even_when_explained(self):
        self.assertEqual(self.verdict(files_removed=["internals.json"], explained=True, allow_added=True), 1)

    def test_loops_are_strict(self):
        self.assertEqual(self.verdict(diffs=[("loops.json", ["-x"])], explained=True, allow_added=True), 1)

    def test_internals_pass_additions_and_changes_with_the_flag_only(self):
        added_changed = ([("x", "fn", "")], [], [("y", "struct", "")])
        self.assertEqual(self.verdict(allow_added=True, internals=added_changed), 0)
        self.assertEqual(self.verdict(explained=True, internals=added_changed), 1)


NO_LOOPS = json.dumps({"loops": []})


def internals(*entries):
    return json.dumps({"internals": [{"path": path, "kind": kind, "hash": hash_, "signature": path, **extra}
                                      for path, kind, hash_, extra in entries]})


A_FN = ("game::a", "fn", "01", {})
TICK = ("game::tick", "fn", "01", {})
GONE = ("game::gone", "fn", "02", {})
B_STRUCT = ("game::B", "struct", "02", {})


class CompareTests(unittest.TestCase):
    def run_compare(self, base_files=None, head_files=None, listed=None, **kwargs):
        return Snapshots(self).run(base_files, head_files, listed=listed or {}, **kwargs)

    def test_same_output_passes(self):
        status, text = self.run_compare({"api/lib.rs": "x"}, {"api/lib.rs": "x"})
        self.assertEqual(status, 0)
        self.assertIn("Mod API unchanged", text)

    def test_the_points_outputs_are_not_compared(self):
        # A base from before #19576 writes them; the head writes none.
        status, text = self.run_compare({"points.json": "[]", "points_generated.rs": "x", "api.rs": "1"},
                                        {"api.rs": "1"})
        self.assertEqual(status, 0, text)

    def test_a_strict_file_the_head_dropped_fails(self):
        status, text = self.run_compare({"internals.json": "{}"}, {"x": "1"}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("the head has to write it and internals.json", text)

    def test_internals_added_only_pass_with_the_flag(self):
        status, text = self.run_compare({"internals.json": internals(A_FN)},
                                        {"internals.json": internals(A_FN, B_STRUCT)}, allow_added=True)
        self.assertEqual(status, 0)
        self.assertIn("Internals added (1):", text)
        self.assertIn("game::B (struct)", text)

    def test_internals_changed_pass_with_the_flag(self):
        status, text = self.run_compare({"internals.json": internals(A_FN, B_STRUCT)},
                                        {"internals.json": internals(A_FN, ("game::B", "struct", "03", {}))},
                                        allow_added=True)
        self.assertEqual(status, 0)
        self.assertIn("Internals changed (1):", text)

    def test_an_unlisted_internals_removal_fails_with_the_flag(self):
        status, text = self.run_compare({"internals.json": internals(A_FN, B_STRUCT)},
                                        {"internals.json": internals(A_FN)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Internals removed and failing (1):", text)
        self.assertIn("removed only when tools/mod_api_removed_internals.txt lists its path", text)

    def test_a_listed_internals_removal_passes(self):
        for allow_added in (False, True):
            with self.subTest(allow_added=allow_added):
                status, text = self.run_compare({"internals.json": internals(A_FN, B_STRUCT)},
                                                {"internals.json": internals(A_FN)}, listed={"game::B": "#9"},
                                                allow_added=allow_added)
                self.assertEqual(status, 0, text)
                self.assertIn("Internals removed and listed in tools/mod_api_removed_internals.txt (accepted) (1):",
                              text)

    def test_a_listing_of_one_path_accepts_only_that_path(self):
        status, text = self.run_compare({"internals.json": internals(A_FN, B_STRUCT)}, {"internals.json": "{\"internals\": []}"},
                                        listed={"game::B": "#9"})
        self.assertEqual(status, 1)
        self.assertIn("Internals removed and failing (1):\n  game::a (fn)", text)

    def test_a_listed_path_the_head_still_has_fails(self):
        for base in (internals(A_FN), internals(A_FN, B_STRUCT)):
            with self.subTest(changed=base != internals(A_FN)):
                status, text = self.run_compare({"internals.json": base}, {"internals.json": internals(A_FN)},
                                                listed={"game::a": "#9", "game::B": "#9"}, allow_added=True)
                self.assertEqual(status, 1)
                self.assertIn("Listed in tools/mod_api_removed_internals.txt but still present (1):\n  game::a", text)

    def test_a_listed_path_present_only_as_shape_only_is_not_stale(self):
        shape = ("game::B", "struct", "02", {"shape_only": True})
        status, text = self.run_compare({"internals.json": internals(A_FN, B_STRUCT)},
                                        {"internals.json": internals(A_FN, shape)}, listed={"game::B": "#9"})
        self.assertEqual(status, 0, text)

    def test_one_of_two_kinds_on_a_path_removed_fails(self):
        field, method = ("game::S::x", "field", "01", {}), ("game::S::x", "method", "02", {})
        status, text = self.run_compare({"internals.json": internals(field, method)},
                                        {"internals.json": internals(method)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("game::S::x (field)", text)

    def test_a_path_that_gains_a_second_kind_fails(self):
        field, method = ("game::S::x", "field", "01", {}), ("game::S::x", "method", "02", {})
        for base, head in ((field, method), (method, field)):
            with self.subTest(base=base[1]):
                status, text = self.run_compare({"internals.json": internals(base)},
                                                {"internals.json": internals(base, head)}, allow_added=True)
                self.assertEqual(status, 1)
                self.assertIn("Internals made ambiguous without a kind (failing) (1):\n  game::S::x", text)
        # A path that already needed a kind, or that only gains a cfg variant, stays as it was.
        gated = ("game::S::x", "field", "01", {"cfg": "unix"})
        for base, head in ((internals(field, method), internals(field, method, gated)),
                           (internals(field), internals(field, gated))):
            status, text = self.run_compare({"internals.json": base}, {"internals.json": head}, allow_added=True)
            self.assertEqual(status, 0, text)

    def test_a_listing_with_a_kind_or_cfg_retires_only_those_entries(self):
        field, method = ("game::S::x", "field", "01", {}), ("game::S::x", "method", "02", {})
        unix, windows = (("game::f", "fn", "01", {"cfg": cfg}) for cfg in ("unix", "windows"))
        base = internals(field, method, unix, windows)
        head = internals(method, unix)
        listed = {("game::S::x", "field", None): "#9", ("game::f", None, "windows"): "#9"}
        status, text = self.run_compare({"internals.json": base}, {"internals.json": head}, listed=listed,
                                        allow_added=True)
        self.assertEqual(status, 0, text)
        # The same listing, kept after the entry comes back, is stale; one that names the kept kind accepts nothing.
        status, text = self.run_compare({"internals.json": base}, {"internals.json": internals(field, method, unix)},
                                        listed=listed, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("still present (1):\n  game::S::x kind=field", text)
        status, _ = self.run_compare({"internals.json": base}, {"internals.json": head},
                                     listed={("game::S::x", "method", None): "#9", "game::f": "#9"}, allow_added=True)
        self.assertEqual(status, 1)

    def test_a_platform_swap_with_the_same_count_fails(self):
        unix, windows, mac = (("game::f", "fn", "01", {"cfg": cfg}) for cfg in ("unix", "windows", "macos"))
        status, text = self.run_compare({"internals.json": internals(unix, windows)},
                                        {"internals.json": internals(unix, mac)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Internals removed and failing (1):", text)
        self.assertIn("game::f (fn, cfg windows)", text)

    def test_becoming_shape_only_fails(self):
        exposed, shape = ("game::T", "struct", "01", {}), ("game::T", "struct", "01", {"shape_only": True})
        status, text = self.run_compare({"internals.json": internals(exposed)},
                                        {"internals.json": internals(shape)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Internals removed and failing (1):", text)
        status, _ = self.run_compare({"internals.json": internals(shape)},
                                     {"internals.json": internals(exposed)}, allow_added=True)
        self.assertEqual(status, 0)

    def test_an_unparsable_internals_difference_fails_with_status_1(self):
        for allow_added in (False, True):
            status, text = self.run_compare({"internals.json": "{}"},
                                            {"internals.json": "{ }"}, allow_added=allow_added)
            self.assertEqual(status, 1)
            self.assertIn("internals.json differs and does not parse as entries (ValueError: no `internals` array",
                          text)

    def test_an_unparsable_internals_keeps_the_other_diffs(self):
        status, text = self.run_compare({"internals.json": "{}", "loops.json": NO_LOOPS},
                                        {"internals.json": "{ }", "loops.json": json.dumps({"loops": [], "x": 1})},
                                        allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("Generated files changed (2): internals.json, loops.json", text)
        self.assertIn("internals.json differs and does not parse as entries", text)
        self.assertIn('+{"loops": [], "x": 1}', text)

    def test_a_dropped_hash_fails(self):
        without_hash = json.loads(internals(A_FN))
        del without_hash["internals"][0]["hash"]
        status, text = self.run_compare({"internals.json": internals(A_FN)},
                                        {"internals.json": json.dumps(without_hash)}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("has no str `hash`", text)

    def test_wrongly_typed_optional_fields_fail(self):
        for extra in ({"cfg": 1}, {"shape_only": "yes"}):
            status, _ = self.run_compare({"internals.json": internals(A_FN)},
                                         {"internals.json": internals(A_FN[:3] + (extra,))}, allow_added=True)
            self.assertEqual(status, 1)

    def test_one_of_two_cfg_variants_removed_fails(self):
        on, off = ("game::f", "mod", "01", {"cfg": "feature = \"f\""}), ("game::f", "mod", "01", {"cfg": "not f"})
        status, _ = self.run_compare({"internals.json": internals(on, off)},
                                     {"internals.json": internals(on)}, allow_added=True)
        self.assertEqual(status, 1)

    def test_any_internals_difference_fails_without_the_flag(self):
        status, text = self.run_compare({"internals.json": internals(A_FN)},
                                        {"internals.json": internals(A_FN, B_STRUCT)})
        self.assertEqual(status, 1)
        self.assertIn("Internals added (1):", text)

    def test_internals_changes_do_not_explain_other_generated_diffs(self):
        status, text = self.run_compare({"internals.json": internals(A_FN), "api.rs": "1"},
                                        {"internals.json": internals(A_FN, B_STRUCT), "api.rs": "2"},
                                        allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("with no added catalog hook", text)

    def test_catalog_hook_addition_preserves_existing_guest_dispatch(self):
        old_hook = {"name": "existing", "neoforge": "existing event"}
        new_hook = {"name": "added", "neoforge": "none"}
        old_arm = ('12 => { let receiver = super::hooks(); let p = crate::__wire_record_52(value);\n'
                   ' let answer = receiver.existing(&p); out.put_owned(answer); }\n')
        new_arm = ('27 => { let receiver = super::hooks(); let p = crate::__wire_record_52(value);\n'
                   ' let answer = receiver.added(&p); out.put_owned(answer); }\n')
        old_name = 'crate::HookId::Existing => "existing",'
        new_name = 'crate::HookId::Added => "added",'
        fallback = '_ => return Err(crate::wire::WireError::Tag(id)) }\n'
        base = {"catalog.json": json.dumps({"hooks": [old_hook]}), "hook-notes.md": NOTES,
                "wire_guest_generated.rs": old_name + "\n" + old_arm + fallback}
        head = {"catalog.json": json.dumps({"hooks": [new_hook, old_hook]}), "hook-notes.md": NOTES.replace("## Existing", "## Added\n\nNew.\n\n## Existing"),
                "wire_guest_generated.rs": new_name + old_name + "\n" + new_arm
                + old_arm.replace("_52", "_53") + fallback}
        for allowed in (True, False):
            with self.subTest(allow_added=allowed):
                status, _ = self.run_compare(base, head, allow_added=allowed)
                self.assertEqual(status, 0 if allowed else 1)
        modified = {**head, "wire_guest_generated.rs":
                    head["wire_guest_generated.rs"].replace("receiver.existing(&p)", "receiver.existing(&other)")}
        status, _ = self.run_compare(base, modified, allow_added=True)
        self.assertEqual(status, 1)
        # A hook appended after the last notes section: the page's separator joins the section before it.
        status, text = self.run_compare(base, {**head, "hook-notes.md": NOTES + "\n## Added\n\nNew.\n"},
                                        allow_added=True)
        self.assertEqual(status, 0, text)

    def test_scoped_part_hook_addition_preserves_existing_guest_dispatch(self):
        old_hook = {"name": "existing", "neoforge": "existing event"}
        new_hook = {"name": "can_place_item", "scope": "block_entity", "neoforge": "none"}
        old_arm = ('12 => { let receiver = super::hooks(); let p = crate::__wire_record_52(value);\n'
                   ' let answer = receiver.existing(&p); out.put_owned(answer); }\n')
        new_arm = ('27 => { let receiver = crate::wire::guest::handler::<dyn crate :: BlockEntityPart>(input.take()?)?;'
                   ' let p = crate::__wire_record_52(value);\n'
                   ' let answer = receiver.can_place_item(&mut p); out.put_owned(answer); }\n')
        old_name = 'crate::HookId::Existing => "existing",'
        new_name = 'crate::HookId::CanPlaceItem => "can_place_item",'
        fallback = '_ => return Err(crate::wire::WireError::Tag(id)) }\n'
        base = {"catalog.json": json.dumps({"hooks": [old_hook]}), "hook-notes.md": NOTES,
                "wire_guest_generated.rs": old_name + "\n" + old_arm + fallback}
        head = {"catalog.json": json.dumps({"hooks": [new_hook, old_hook]}), "hook-notes.md": NOTES,
                "wire_guest_generated.rs": new_name + old_name + "\n" + new_arm
                + old_arm.replace("_52", "_53") + fallback}
        for allowed in (True, False):
            with self.subTest(allow_added=allowed):
                status, text = self.run_compare(base, head, allow_added=allowed)
                self.assertEqual(status, 0 if allowed else 1, text)
        # An added hook explains new note sections, not an edit to an existing hook's or the general notes.
        for notes in (NOTES.replace("Old.", "Edited.") + "\n## Added\n", NOTES.replace("General.", "Edited.")
                      + "\n## Added\n", "# Hook notes\n\n## Added\n"):
            with self.subTest(notes=notes):
                status, _ = self.run_compare(base, {**head, "hook-notes.md": notes}, allow_added=True)
                self.assertEqual(status, 1)

    def guest_dispatch(self, records, added=None, added_name=False):
        """A guest wire around the arms of #18539's log: `records` numbers the preceding arm, effect_particle_modification,
        and enchantment_level_set's nested and outer record; `added` is an arm put after the first."""
        before, effect, nested, outer = records
        names = ('crate::HookId::DimensionTransitionScreenDraw => "dimension_transition_screen_draw",'
                 + ('crate::HookId::DragonFightWon => "dragon_fight_won",' if added_name else "")
                 + 'crate::HookId::EffectParticleModification => "effect_particle_modification",')
        arms = [
            ('1914093287 => { let receiver = super::hooks(); let p1_0: f32 = input.take()?;\n'
             f'let mut p1 = crate::__wire_record_{before}(p1_0);\n'
             ' let answer = receiver.dimension_transition_screen_draw(&host, &mut p1); out.put_owned(answer);'
             ' out.put(&p1.reason);out.put(&p1.progress); }\n'),
            ('808237270 => { let receiver = super::hooks(); let p1_0: mod_api::LivingRef = input.take()?;\n'
             'let p1_4: bool = input.take()?;\n'
             f'let mut p1 = crate::__wire_record_{effect}(p1_0, &p1_1, &p1_2, &mut p1_3, p1_4);\n'
             ' let answer = receiver.effect_particle_modification(&host, &mut p1); out.put_owned(answer);'
             ' out.put(&p1.entity);out.put(&p1.visible);out.put(&*p1.particle); }\n'),
            ('2034981 => { let receiver = super::hooks(); let p1_1_5: i32 = input.take()?;\n'
             f'let p1_1 = crate::__wire_record_{nested}(p1_1_0, p1_1_1, p1_1_2, p1_1_3, &p1_1_4, p1_1_5);'
             'let p1_2: i32 = input.take()?;\n'
             f'let mut p1 = crate::__wire_record_{outer}(p1_0, &p1_1, p1_2);\n'
             ' let answer = receiver.enchantment_level_set(&mut host, &mut p1); out.put_owned(answer);'
             ' out.put(&p1.pos);out.put(&p1.level); }\n'),
        ]
        if added:
            arms.insert(1, added)
        return (f"pub fn hook_name(id: crate::HookId) -> &'static str {{ match id {{ {names} }} }}\n"
                "// @generated by horizon-mod-api-gen; do not edit.\n\npub fn hook(id: u32, input: &mut "
                "crate::wire::Reader<'_>, out: &mut crate::wire::Writer) -> Result<(), crate::wire::WireError> {\n"
                "let mut host = Guest;\nmatch id {\n" + "".join(arms)
                + "_ => return Err(crate::wire::WireError::Tag(id)) }\nOk(())\n}\n")

    # #18539's arm, as logged: its event is owned, so it names no record, but its type still took number 858.
    OWNED_ARM = ('534749878 => { let receiver = super::hooks(); let mut p1: mod_api::DragonFightWonEvent = '
                 'input.take()?;\n let answer = receiver.dragon_fight_won(&host, &mut p1); out.put_owned(answer); '
                 'out.put(&p1); }\n')
    BORROWED_ARM = ('534749878 => { let receiver = super::hooks(); let p1_0: i32 = input.take()?;\n'
                    'let mut p1 = crate::__wire_record_858(p1_0);\n let answer = receiver.dragon_fight_won(&host, '
                    '&mut p1); out.put_owned(answer); out.put(&p1.level); }\n')

    def compare_hook_addition(self, head_records, added_arm, edit=lambda text: text):
        """The gate's status for the base's dispatch against one with `added_arm` and `head_records`."""
        old_hook = {"name": "dimension_transition_screen_draw", "neoforge": "none"}
        new_hook = {"name": "dragon_fight_won", "neoforge": "none"}
        base = {"catalog.json": json.dumps({"hooks": [old_hook]}),
                "wire_guest_generated.rs": self.guest_dispatch((857, 858, 862, 861))}
        head = {"catalog.json": json.dumps({"hooks": [old_hook, new_hook]}),
                "wire_guest_generated.rs": edit(self.guest_dispatch(head_records, added_arm, added_name=True))}
        return self.run_compare(base, head, allow_added=True)[0]

    def test_an_added_hook_with_a_borrowed_event_passes(self):
        # #17989's case: the new arm names its own record, and the later ones move up by one.
        self.assertEqual(self.compare_hook_addition((857, 859, 863, 862), self.BORROWED_ARM), 0)

    def test_an_added_hook_with_an_owned_event_passes(self):
        # #18539's case: the number the event took is named nowhere, yet the later records are one higher.
        self.assertEqual(self.compare_hook_addition((857, 859, 863, 862), self.OWNED_ARM), 0)
        with self.subTest("the event and a new owned type only it uses: two numbers"):
            self.assertEqual(self.compare_hook_addition((857, 860, 864, 863), self.OWNED_ARM), 0)

    def test_an_owned_type_an_existing_hook_gains_beside_an_added_hook_is_not_seen_here(self):
        # Not coverage: dimension_transition_screen_draw's answer going from () to a new owned type takes 858 and
        # leaves its arm's text as it was (`out.put_owned(answer)`), and the added hook's numbers absorb it. This
        # check passes it; the surface report fails it (the hook gains an output).
        self.assertEqual(self.compare_hook_addition((857, 860, 864, 863), self.OWNED_ARM), 0)
        borrowed = self.BORROWED_ARM.replace("__wire_record_858", "__wire_record_859")
        self.assertEqual(self.compare_hook_addition((857, 860, 864, 863), borrowed), 0)

    def test_an_added_hook_that_first_uses_an_existing_owned_type_fails(self):
        # Additive, but refused: the added hook is now the first to use an owned type effect_particle_modification
        # used at 859, so effect's record moves by two and the later ones by one. A number missing after the window
        # can't be told from a later hook losing an owned type, so this fails rather than open that.
        self.assertEqual(self.compare_hook_addition((857, 860, 863, 862), self.OWNED_ARM), 1)

    def test_an_added_hook_and_a_new_host_area_with_its_own_records_pass(self):
        # The three retained records above the new host import shift even before the hook.
        records = (859, 861, 865, 864)
        source = ('impl crate::VideoClientHost for Guest {\n'
                  'fn start(&mut self) -> u32 { let r = host_import!("Video::start#ab", 5u32, w); r.take() }\n'
                  '}\n')
        self.assertEqual(self.compare_hook_addition(records, self.OWNED_ARM, lambda text: source + text), 0)
        unsupported = source.replace('}\n', 'fn unexpected(&self) {}\n}\n', 1)
        self.assertEqual(self.compare_hook_addition(records, self.OWNED_ARM, lambda text: unsupported + text), 1)
        self.assertEqual(self.compare_hook_addition((859, 861, 864, 865), self.OWNED_ARM,
                                                    lambda text: source + text), 1)

    def test_a_changed_existing_arm_beside_an_added_hook_fails(self):
        cases = {
            "an existing arm's text": ((857, 859, 863, 862), lambda text: text.replace("out.put(&p1.visible);", "")),
            "a shift before the added arm": ((858, 859, 863, 862), lambda text: text),
            "a shift after the next arm": ((857, 858, 863, 862), lambda text: text),
            "two retained records swapped": ((857, 859, 862, 863), lambda text: text),
            "a retained record merged into another": ((857, 859, 863, 859), lambda text: text),
        }
        for name, (records, edit) in cases.items():
            for arm in (self.OWNED_ARM, self.BORROWED_ARM):
                with self.subTest(name, borrowed=arm is self.BORROWED_ARM):
                    self.assertEqual(self.compare_hook_addition(records, arm, edit), 1)

    @staticmethod
    def forwarders(calls, records=(40, 41)):
        """A guest wire's `Guest` impl with one forwarder per (name, number) call, then a hook arm naming two records."""
        first, second = records
        lines = [f'fn {name.split("#")[0].split("::")[1]}(&self) -> u32 {{ let mut w = crate::wire::guest::begin(); '
                 f'let r = host_import!("{name}", {number}u32, w); r.take().expect("wrong type") }}\n'
                 for name, number in calls]
        return ("impl mod_api::prelude::PlayersServerHost for Guest {\n" + "".join(lines) + "}\n"
                "pub fn hook(id: u32, input: &mut crate::wire::Reader<'_>, out: &mut crate::wire::Writer) {\nmatch id {\n"
                f"7 => {{ let receiver = super::hooks(); let p = crate::__wire_record_{first}(input.take()?);\n"
                f" let q = mod_api::__wire_record_{second}(&p); let answer = receiver.existing(&host, &q); }}\n"
                "_ => return Err(crate::wire::WireError::Tag(id)) }\n}\n")

    BASE_CALLS = [("Players::online#aa", 1091), ("Players::kick#bb", 1092), ("Chunk::forced#cc", 1100)]

    def compare_call_addition(self, head_calls, records=(40, 41), allow_added=True):
        base = {"catalog.json": json.dumps({"hooks": [{"name": "existing"}]}),
                "wire_guest_generated.rs": self.forwarders(self.BASE_CALLS)}
        head = {"catalog.json": base["catalog.json"], "wire_guest_generated.rs": self.forwarders(head_calls, records)}
        return self.run_compare(base, head, allow_added=allow_added)

    def test_an_added_host_call_passes_with_the_flag_only(self):
        # The new call takes position 1092, so the later imports, held ones too, move up by one.
        head = [("Players::online#aa", 1091), ("Players::raw#dd", 1092), ("Players::kick#bb", 1093),
                ("Chunk::forced#cc", 1101)]
        status, text = self.compare_call_addition(head)
        self.assertEqual(status, 0, text)
        self.assertIn("Host calls added (accepted): Players::raw#dd", text)
        self.assertEqual(self.compare_call_addition(head, allow_added=False)[0], 1)
        with self.subTest("a type the call brings in moves the later records"):
            self.assertEqual(self.compare_call_addition(head, records=(40, 43))[0], 0)
            self.assertEqual(self.compare_call_addition(head, records=(42, 43))[0], 0)

    def test_a_changed_or_removed_host_call_beside_an_added_one_fails(self):
        added = ("Players::raw#dd", 1092)
        cases = {
            "a changed signature": [("Players::online#ee", 1091), added, ("Players::kick#bb", 1093),
                                    ("Chunk::forced#cc", 1101)],
            "a removed call": [("Players::online#aa", 1091), added, ("Chunk::forced#cc", 1100)],
        }
        for name, calls in cases.items():
            with self.subTest(name):
                self.assertEqual(self.compare_call_addition(calls)[0], 1)
        with self.subTest("two retained records swapped"):
            calls = [("Players::online#aa", 1091), added, ("Players::kick#bb", 1093), ("Chunk::forced#cc", 1101)]
            self.assertEqual(self.compare_call_addition(calls, records=(43, 42))[0], 1)

    def test_an_added_host_call_does_not_explain_other_edits(self):
        base_calls = self.BASE_CALLS
        head = self.forwarders([("Players::online#aa", 1091), ("Players::raw#dd", 1092), ("Players::kick#bb", 1093),
                                ("Chunk::forced#cc", 1101)])
        base = {"wire_guest_generated.rs": self.forwarders(base_calls), "hook-notes.md": NOTES}
        for name, files in {
            "an edited forwarder": {"wire_guest_generated.rs": head.replace('1093u32, w); r.take().expect("wrong type")',
                                                                            '1093u32, w); r.take().expect("other")')},
            "an edited hook note": {"wire_guest_generated.rs": head, "hook-notes.md": NOTES.replace("Old.", "New.")},
        }.items():
            with self.subTest(name):
                status, text = self.run_compare(base, {"hook-notes.md": NOTES, **files}, allow_added=True)
                self.assertEqual(status, 1, text)

    def test_an_added_native_only_unsupported_stub_passes_with_the_flag(self):
        stub = ('#[cfg(not(target_arch = "wasm32"))]\n'
                'fn tool_break_block(&mut self, p0: u32) -> bool '
                '{ crate::wire::guest::unsupported("BackendMut::tool_break_block") }\n')
        existing_stub = ('#[cfg(not(target_arch = "wasm32"))]\n'
                         'fn existing_native(&self) -> bool '
                         '{ crate::wire::guest::unsupported("BackendMut::existing_native") }\n')
        base_wire = self.forwarders(self.BASE_CALLS).replace("\n}\n", "\n" + existing_stub + "}\n", 1)
        head_wire = base_wire.replace("\n}\n", "\n" + stub + "}\n", 1)
        status, text = self.run_compare({"wire_guest_generated.rs": base_wire},
                                        {"wire_guest_generated.rs": head_wire}, allow_added=True)
        self.assertEqual(status, 0, text)
        self.assertIn("Host calls added (accepted): BackendMut::tool_break_block", text)
        self.assertEqual(self.run_compare({"wire_guest_generated.rs": base_wire},
                                          {"wire_guest_generated.rs": head_wire}, allow_added=False)[0], 1)
        # Editing an existing unsupported stub still fails.
        edited = head_wire.replace("fn existing_native(&self) -> bool", "fn existing_native(&self) -> u32")
        self.assertEqual(self.run_compare({"wire_guest_generated.rs": base_wire},
                                          {"wire_guest_generated.rs": edited}, allow_added=True)[0], 1)

    def test_a_loops_json_difference_fails_with_the_flag(self):
        status, _ = self.run_compare({"loops.json": NO_LOOPS}, {"loops.json": json.dumps({"loops": [], "x": 1})}, allow_added=True)
        self.assertEqual(status, 1)

    def test_a_strict_file_only_the_head_wrote_is_not_compared(self):
        status, text = self.run_compare(None, {"loops.json": "{}"})
        self.assertEqual(status, 0)
        self.assertIn("loops.json exists on one side only", text)

    def test_file_diff_is_printed_and_capped(self):
        old = "\n".join(f"line {i}" for i in range(500))
        new = "\n".join(f"other {i}" for i in range(500))
        status, text = self.run_compare({"catalog.json": old}, {"catalog.json": new})
        self.assertEqual(status, 1)
        self.assertIn("Generated files changed (1): catalog.json", text)
        self.assertIn("more lines of catalog.json left out", text)
        self.assertLess(len(text.splitlines()), gate.MAX_DIFF_LINES + 25)

    def test_a_generated_diff_without_an_added_hook_fails_with_the_flag(self):
        status, text = self.run_compare({"api.rs": "1"}, {"api.rs": "2"}, allow_added=True)
        self.assertEqual(status, 1)
        self.assertIn("with no added catalog hook", text)

    def test_generator_scratch_is_not_compared(self):
        base = {"interfaces-aaa/1.cache": "x", "points-timing.txt": "a 1.00s\n", "api.rs": "1"}
        head = {"interfaces-bbb/2.cache": "y", "interfaces-bbb/3.cache": "z", "incremental-ccc/4.entry": "w",
                "points-timing.txt": "a 2.00s\n", "api.rs": "1"}
        status, text = self.run_compare(base, head)
        self.assertEqual(status, 0)
        self.assertIn("Mod API unchanged", text)

    def test_only_the_cache_folder_and_timing_file_are_scratch(self):
        status, _ = self.run_compare({"api/points-timing.txt": "1"},
                                     {"api/points-timing.txt": "2"})
        self.assertEqual(status, 0)
        status, _ = self.run_compare({"interfaces.rs": "1"}, {"interfaces.rs": "2"})
        self.assertEqual(status, 1)

    def test_source_folder_paths_do_not_count_as_a_difference(self):
        status, _ = self.run_compare({"hook-notes.md": "see /tmp/base/x"},
                                     {"hook-notes.md": "see /work/head/x"}, base_root="/tmp/base",
                                     head_root="/work/head")
        self.assertEqual(status, 0)

    def test_summary_lists_are_capped(self):
        added = [(f"game::p{i:03}", "fn", "") for i in range(100)]
        lines = gate.summary([], [], [], internals=(added, [], []))
        self.assertIn("Internals added (100):", lines[0])
        self.assertIn(f"... and {100 - gate.MAX_LISTED} more", lines[-1])


class RemovedListTests(unittest.TestCase):
    def read(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "list.txt")
            path.write_text(text)
            return gate.load_removed(path)

    def test_lines_and_comments(self):
        self.assertEqual(self.read("# header\n\nhorizon_x::A  #19576\nhorizon_x::B #2\n"),
                         {"horizon_x::A": "#19576", "horizon_x::B": "#2"})

    def test_a_kind_and_a_cfg_narrow_a_line(self):
        self.assertEqual(self.read("horizon_x::A  kind=field  #1\nhorizon_x::A  cfg=feature = \"f\"  #2\n"
                                   "horizon_x::B  kind=fn  cfg=  #3\n"),
                         {("horizon_x::A", "field", None): "#1", ("horizon_x::A", None, 'feature = "f"'): "#2",
                          ("horizon_x::B", "fn", ""): "#3"})

    def test_a_line_without_an_issue_or_listed_twice_is_refused(self):
        for text in ("horizon_x::A\n", "horizon_x::A  19576\n", "horizon_x::A  #1  moved\n",
                     "horizon_x::A  #1\nhorizon_x::A  #2\n", "horizon_x::A  cfg=unix  kind=fn  #1\n",
                     "horizon_x::A  kind=fn  #1\nhorizon_x::A  kind=fn  #2\n"):
            with self.subTest(text), self.assertRaises(ValueError):
                self.read(text)

    def test_the_checked_in_file_parses(self):
        self.assertIsInstance(gate.load_removed(gate.ROOT / gate.REMOVED), dict)


class TargetFolderTests(unittest.TestCase):
    def test_a_relative_folder_becomes_one_absolute_path(self):
        for option, env in ((Path("target/mod-api"), None), (None, "target/x")):
            self.assertTrue(gate.resolve_target(option, env).is_absolute())
        self.assertEqual(gate.resolve_target(Path("a"), "b"), Path("a").resolve())
        self.assertEqual(gate.resolve_target(None, None), (gate.ROOT / "target").resolve())

    def test_the_base_builds_in_its_own_folder_inside_the_working_trees(self):
        target = Path("/build/target")
        self.assertEqual(gate.base_target(target), target / "mod-api-base")
        self.assertNotEqual(gate.base_target(target), target)


class SnapshotStoreTests(unittest.TestCase):
    """A stored snapshot of the base stands in for its build only when complete and under the same key."""

    RUSTC = "rustc 1.99.0 (abc 2026-11-01)\nhost: x86_64-unknown-linux-gnu"

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.store = self.tmp / "store"
        self.key = gate.snapshot_key("c0ffee", self.RUSTC, env={})

    def made(self, name, root):
        """A snapshot folder as hook_snapshot writes it, naming `root` in a generated file."""
        out = self.tmp / name
        out.mkdir()
        (out / "internals.json").write_text(internals(TICK))
        (out / "api.rs").write_text(f"// generated from {root}/crates/horizon-client/src/game.rs\n")
        return out

    def stored(self, key=None):
        self.made("main-out", "/work/main")
        gate.store_snapshot(self.store, self.tmp / "main-out", key or self.key, "/work/main")
        return self.store / (key or self.key)["commit"]

    def test_a_stored_snapshot_compares_as_the_built_base_would(self):
        self.stored()
        folder, root, route = gate.find_snapshot(self.store, self.key)
        self.assertEqual(route, "hit")
        self.assertEqual(root, "/work/main")
        head = self.made("head-out", "/work/head")
        built = self.made("built-out", "/tmp/base")
        with contextlib.redirect_stdout(io.StringIO()) as hit:
            hit_status = gate.compare(folder, head, base_root=root, head_root="/work/head")
        with contextlib.redirect_stdout(io.StringIO()) as build:
            build_status = gate.compare(built, head, base_root="/tmp/base", head_root="/work/head")
        self.assertEqual((hit_status, hit.getvalue()), (build_status, build.getvalue()))
        self.assertEqual(hit_status, 0)

    def test_nothing_stored_is_a_miss(self):
        self.assertEqual(gate.find_snapshot(self.store, self.key), (None, None, "miss: none stored"))

    def test_another_toolchain_or_point_name_selection_is_a_miss(self):
        self.stored()
        for key, differs in ((gate.snapshot_key("c0ffee", "rustc 2.0.0", env={}), "rustc"),
                             (gate.snapshot_key("c0ffee", self.RUSTC, env={"HORIZON_POINT_NAMES": '["tick"]'}),
                              "point_names")):
            self.assertEqual(gate.find_snapshot(self.store, key),
                             (None, None, f"miss: stored under another key ({differs})"))

    def remarked(self, folder, digest=True, **fields):
        """Rewrite the marker with `fields` changed, its digest recomputed or (digest=False) left as it was."""
        marker = json.loads((folder / gate.MARKER).read_text())
        marker.update(fields)
        if digest:
            marker["digest"] = gate.marker_digest(marker["key"], marker["root"], marker["files"])
        (folder / gate.MARKER).write_text(json.dumps(marker))

    def test_an_incomplete_or_corrupt_snapshot_is_a_miss_not_an_error(self):
        damages = {
            "truncated file": lambda folder: (folder / "out/internals.json").write_text("{\"internals\": ["),
            "added file": lambda folder: (folder / "out/extra.json").write_text("{}"),
            "removed file": lambda folder: (folder / "out/api.rs").unlink(),
            "no marker": lambda folder: (folder / gate.MARKER).unlink(),
            "garbled marker": lambda folder: (folder / gate.MARKER).write_text("{\"key\": "),
            "damaged root": lambda folder: self.remarked(folder, digest=False, root="/work/mail"),
            "damaged key": lambda folder: self.remarked(folder, digest=False, key=dict(self.key, rustc="x")),
            "marker of another commit": lambda folder: self.remarked(folder, key=dict(self.key, commit="beef")),
            "marker without digest": lambda folder: (folder / gate.MARKER).write_text(
                json.dumps({"key": self.key, "root": "/work/main", "files": gate.file_hashes(folder / "out")})),
        }
        for name, damage in damages.items():
            with self.subTest(name):
                shutil.rmtree(self.store, ignore_errors=True)
                shutil.rmtree(self.tmp / "main-out", ignore_errors=True)
                damage(self.stored())
                folder, root, route = gate.find_snapshot(self.store, self.key)
                self.assertIsNone(folder)
                self.assertTrue(route.startswith("miss: "), route)

    def test_only_the_newest_snapshots_and_no_partial_ones_are_kept(self):
        for number in range(4):
            out = self.tmp / f"out{number}"
            out.mkdir()
            (out / "internals.json").write_text("{}")
            gate.store_snapshot(self.store, out, dict(self.key, commit=f"c{number}"), "/r", keep=10)
            marker = self.store / f"c{number}" / gate.MARKER
            os.utime(marker, (1000 + number, 1000 + number))
        (self.store / f"{gate.TEMPORARY}c9-1").mkdir()
        gate.prune(self.store, keep=2)
        self.assertEqual(sorted(path.name for path in self.store.iterdir()), ["c2", "c3"])

    def test_a_pull_request_never_stores(self):
        self.assertFalse(gate.may_store({"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "pull_request"}))
        self.assertFalse(gate.may_store({"SAVE_BUILD_DISK": "false", "GITHUB_EVENT_NAME": "workflow_dispatch"}))
        self.assertFalse(gate.may_store({}))
        self.assertTrue(gate.may_store({"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "push"}))
        self.assertTrue(gate.may_store({"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "merge_group"}))
        built = mock.Mock(side_effect=AssertionError("built"))
        with mock.patch.dict(os.environ, {"SAVE_BUILD_DISK": "true", "GITHUB_EVENT_NAME": "pull_request"}), \
                mock.patch.object(gate, "cargo", built), contextlib.redirect_stdout(io.StringIO()):
            status = gate.main(["--store-snapshot", str(self.store)])
        self.assertEqual(status, 2)
        self.assertFalse(self.store.exists())

    def test_a_failed_store_leaves_no_partial_copy(self):
        out = self.made("main-out", "/work/main")
        with mock.patch.object(gate, "file_hashes", mock.Mock(side_effect=OSError("No space left on device"))):
            with self.assertRaises(OSError):
                gate.store_snapshot(self.store, out, self.key, "/work/main")
        self.assertEqual(list(self.store.iterdir()), [])

    def check(self, base_internals, base_files=None, checkout_files=None):
        """main() comparing a working tree of [tick] with the base `self.key` names, the stored snapshot (if any)
        holding [tick] as well: (status, the sides built, the output). Git and the generator are stand-ins; a built
        base writes `base_internals` (internals.json entries). `base_files` are the base commit's files ({name: text}) and `checkout_files` those
        written into the checkout."""
        sides, base_files = [], base_files or {}
        checkout = self.tmp / "checkout"
        shutil.rmtree(checkout, ignore_errors=True)
        checkout.mkdir()
        for name, text in (checkout_files or {}).items():
            (checkout / name).write_text(text)

        def snapshot(root, out, target_dir, side):
            sides.append(side)
            out.mkdir()
            entries = base_internals if side == "base" else [TICK]
            (out / "internals.json").write_text(internals(*entries))
            (out / "api.rs").write_text(f"// generated from {root}/crates/horizon-client/src/game.rs\n")
            return 1.0, 2.0

        def git(*args, cwd=gate.ROOT):
            if args[0] == "rev-parse":
                return self.key["commit"]
            if args[0] == "ls-tree":
                return args[-1] if args[-1] in base_files else ""
            if args[0] == "show":
                return base_files[args[1].split(":", 1)[1]].strip()
            self.assertEqual(args[:2], ("worktree", "add"))
            Path(args[-2]).mkdir(parents=True)
            return ""

        # No real git: `git ls-files` lists nothing and the worktree removal does nothing.
        listed = gate.subprocess.CompletedProcess([], 0, "", "")
        with mock.patch.object(gate, "snapshot", snapshot), mock.patch.object(gate, "git", git), \
                mock.patch.object(gate, "toolchain", lambda root: self.RUSTC), \
                mock.patch.object(gate, "ROOT", checkout), \
                mock.patch.object(gate.subprocess, "run", return_value=listed), \
                contextlib.redirect_stdout(io.StringIO()) as out:
            status = gate.main(["--base", "HEAD", "--snapshots", str(self.store), "--target-dir", str(self.tmp)])
        return status, sides, out.getvalue()

    def test_a_hit_builds_only_the_working_tree(self):
        self.key = gate.snapshot_key("c0ffee", self.RUSTC)
        self.stored()
        status, sides, out = self.check([TICK, GONE])
        self.assertEqual((status, sides), (0, ["working tree"]), out)
        self.assertIn("Mod API base snapshot: hit; base 0 s (stored)", out)

    def test_a_miss_builds_the_base_and_compares_with_it(self):
        # The built base has an internal the working tree lost: only a comparison with the built base fails.
        self.key = gate.snapshot_key("c0ffee", self.RUSTC)
        for name, prepare, route in (
                ("nothing stored", lambda: None, "miss: none stored"),
                ("corrupt root", lambda: self.remarked(self.stored(), digest=False, root="/work/mail"),
                 "miss: incomplete (the marker does not match its digest)")):
            with self.subTest(name):
                shutil.rmtree(self.store, ignore_errors=True)
                shutil.rmtree(self.tmp / "main-out", ignore_errors=True)
                prepare()
                status, sides, out = self.check([TICK, GONE])
                self.assertEqual((status, sides), (1, ["base", "working tree"]), out)
                self.assertIn(f"Mod API base snapshot: {route}; base 3 s (compile 1 s, generator 2 s)", out)
                self.assertIn("Internals removed and failing (1):", out)

    def test_a_toolchain_only_the_change_selects_never_hits(self):
        # The probe runs in the checkout, so a pin there could match a snapshot the base's compiler never made.
        self.key = gate.snapshot_key("c0ffee", self.RUSTC)
        pin = '[toolchain]\nchannel = "1.99.0"\n'
        for name, base_files, checkout_files in (
                ("added", {}, {"rust-toolchain.toml": pin}),
                ("changed", {"rust-toolchain": "1.98.0"}, {"rust-toolchain": "1.99.0"}),
                ("removed", {"rust-toolchain.toml": pin}, {})):
            with self.subTest(name):
                shutil.rmtree(self.store, ignore_errors=True)
                shutil.rmtree(self.tmp / "main-out", ignore_errors=True)
                self.stored()
                status, sides, out = self.check([TICK, GONE], base_files, checkout_files)
                self.assertEqual((status, sides), (1, ["base", "working tree"]), out)
                self.assertIn("Mod API base snapshot: miss: the change selects another toolchain;", out)
        with self.subTest("the same pin"):
            status, sides, out = self.check([TICK], {"rust-toolchain.toml": pin}, {"rust-toolchain.toml": pin})
            self.assertEqual((status, sides), (0, ["working tree"]), out)


class ToolchainFilesTests(unittest.TestCase):
    """selects_base_toolchain on a real repository: the checkout's toolchain files against a commit's."""

    def test_a_file_the_change_adds_edits_or_removes_selects_another_toolchain(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def commit(message):
                for args in (["add", "-A"], ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                                             "commit", "-q", "--allow-empty", "-m", message]):
                    subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
                return gate.git("rev-parse", "HEAD", cwd=root)

            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            (root / "lib.rs").write_text("")
            plain = commit("no pin")
            self.assertTrue(gate.selects_base_toolchain(plain, root))
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.99.0"\n')
            self.assertFalse(gate.selects_base_toolchain(plain, root), "untracked pin")
            pinned = commit("pin")
            self.assertTrue(gate.selects_base_toolchain(pinned, root))
            self.assertFalse(gate.selects_base_toolchain(plain, root), "committed pin")
            (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.0"\n')
            self.assertFalse(gate.selects_base_toolchain(pinned, root), "edited pin")
            (root / "rust-toolchain.toml").unlink()
            self.assertFalse(gate.selects_base_toolchain(pinned, root), "removed pin")


if __name__ == "__main__":
    unittest.main()
