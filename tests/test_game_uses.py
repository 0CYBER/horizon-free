from pathlib import Path
import tempfile
import unittest

from tools import game_uses

GAME = '''
struct Game<'a> {
    mobs: Vec<u32>,
    world: World,
    next_entity: u32,
    ui: Ui,
    log: Vec<String>,
    level: Level,
}

impl Game<'_> {
    /// self.ui in a comment is not a use
    pub(super) fn tick(&mut self, n: u32) -> u32 {
        let text = "self.log.push(1)";
        let c = 'x';
        for m in self.mobs.iter() {
            self.world.random.next_float();
        }
        self.mobs.push(n);
        self.next_entity += 1;
        let r = &mut self.world;
        let d = self.ui.difficulty + self.ui.level;
        self.log[0] = text.to_string();
        self.helper(&mut self.mobs, 1);
        self.spawn_item(n);
        self.send_packet(Packet::Ping);
        Self::assoc(3);
        MobEvent::Spawn { id: Foo::new() };
        free_fn(self);
        0
    }
    fn helper(&self, a: &mut Vec<u32>, b: u32) {}
    fn spawn_item(&mut self, n: u32) {}
    fn send_packet(&mut self, p: Packet) {}
    fn by_value(self) {}
    fn assoc(n: u32) {}
    fn takes_impl(&mut self, f: impl FnOnce()) {}
    fn takes_result(&self, result: u32) {}
    fn native_tick(&mut self) {}
    #[inline(always)]
    fn forced(&self) {}
    #[cfg(test)]
    fn only_test(&self) {}
    fn gives_impl(&self) -> impl Iterator<Item = u32> { std::iter::empty() }
    fn ok_ref_impl(&self, f: &impl Fn()) {}
    unsafe fn risky(&self) {}
    fn gives_nl(&self) -> impl
        Iterator<Item = u32> { std::iter::empty() }
    fn apply<F: Fn() -> u32>(&self, f: F) {}
    fn reset(&mut self) { self.mobs.clear(); }
    fn via_self(&mut self) { Self::reset(self); Game::reset(self); let _ = Self::LIMIT; }
    fn draws(&mut self) {
        let p = self.world.rng.next_float() - self.world.rng.next_float();
        self.send_packet(p); self.next_entity += 1;
    }
    fn copies(&mut self) {
        self.mobs.clone().clear();
        self.mobs.as_mut().clear();
        self.level.set_difficulty(1);
        crate::game::helper();
    }
}

impl Trait for Game<'_> {
    fn on_trait(&mut self) {}
}

impl Other {
    fn tick(&self) { self.mobs.len(); }
}
'''


class GameUsesTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        root = Path(cls.tmp.name).resolve()
        (root / "game.rs").write_text(GAME)
        cls.index = game_uses.Index(root, root)
        cls.src = cls.index.sources[root / "game.rs"]

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def uses(self, name="tick"):
        (_, src, start, end, fn), *_ = self.index.find(f"game.rs::{name}")  # the Game one first
        return game_uses.analyse(self.index, src, start, end, fn)

    def test_game_fields_are_read_from_the_struct(self):
        self.assertEqual(self.index.fields, ["mobs", "world", "next_entity", "ui", "log", "level"])

    def test_fields_split_into_reads_and_writes(self):
        fields = self.uses().fields
        # mobs: `.iter()` reads; `.push(n)` and `&mut self.mobs` write
        self.assertEqual((fields["mobs"]["read"], fields["mobs"]["write"]), (1, 2))
        self.assertEqual(fields["next_entity"], {"read": 0, "write": 1, "lines": fields["next_entity"]["lines"]})
        self.assertEqual(fields["ui"]["read"], 2)
        self.assertEqual(fields["ui"]["write"], 0)
        self.assertEqual(fields["log"]["write"], 1)
        self.assertEqual(fields["world"]["write"], 1)  # `&mut self.world`
        self.assertEqual(fields["world"]["read"], 1)  # `.random.next_float()`

    def test_comments_and_strings_are_not_uses(self):
        self.assertNotIn("push(1)", " ".join(self.uses().methods))
        self.assertEqual(self.uses().fields["log"]["write"], 1)

    def test_called_methods_and_whole_self(self):
        uses = self.uses()
        self.assertEqual(set(uses.methods), {"helper", "spawn_item", "send_packet", "assoc"})
        self.assertEqual(set(uses.assoc), set())
        self.assertEqual(len(uses.whole_self), 1)
        self.assertIn("free_fn(self)", uses.whole_self[0])

    def test_markers_keep_their_lines(self):
        markers = {(m["category"], m["match"]) for m in self.uses().markers}
        self.assertIn(("random", "random"), markers)
        self.assertIn(("random", "next_float"), markers)
        self.assertIn(("entity id", "next_entity"), markers)
        self.assertIn(("spawn", "spawn_item"), markers)
        self.assertIn(("send", "send_packet("), markers)
        lines = [m["line"] for m in self.uses().markers]
        self.assertEqual(lines, sorted(lines))

    def test_named_items_fold_enum_members(self):
        names = set(self.uses().names)
        self.assertIn("MobEvent::Spawn", names)
        self.assertIn("Foo::new", names)
        self.assertIn("free_fn", names)

    def candidate(self, name):
        (found,) = [f for f in self.index.methods[name]]
        return found.eligible()

    def test_point_eligibility_follows_the_shape_rule(self):
        self.assertTrue(self.candidate("tick")[0])
        self.assertTrue(self.candidate("ok_ref_impl")[0])  # `&impl Fn()` is a reference, not `impl Trait`
        for name, word in [("by_value", "receiver"), ("assoc", "receiver"), ("takes_impl", "impl Trait"),
                           ("takes_result", "result"), ("native_tick", "native_"), ("forced", "inline(always)"),
                           ("only_test", "cfg"), ("gives_impl", "returns impl"), ("gives_nl", "returns impl"),
                           ("risky", "unsafe")]:
            ok, why = self.candidate(name)
            self.assertFalse(ok, name)
            self.assertIn(word, why, name)

    def test_trait_impls_and_other_types_are_kept_apart(self):
        self.assertNotIn("on_trait", self.index.methods)
        self.assertIn("on_trait", self.index.trait_methods)
        self.assertEqual(len(self.index.methods["tick"]), 1)  # not `impl Other`'s `tick`
        info = game_uses.method_info(self.index, "on_trait")
        self.assertFalse(info["candidate"])

    def test_line_range_spec(self):
        first = self.src.text[:self.src.text.index("self.world.random")].count("\n") + 1
        (_, src, start, end, fn), = self.index.find(f"game.rs:{first}-{first}")
        self.assertIsNone(fn)
        uses = game_uses.analyse(self.index, src, start, end, None)
        self.assertEqual(set(uses.fields), {"world"})


    def test_methods_of_another_type_are_not_attributed_to_game(self):
        """`game.rs::tick` also matches `Other::tick`, whose `self.mobs` is not the game's."""
        found = self.index.find("game.rs::tick")
        self.assertEqual([f[4].owner for f in found], ["Game", "Other"])
        _, src, start, end, fn = found[1]
        uses = game_uses.analyse(self.index, src, start, end, fn)
        self.assertEqual(uses.fields, {})
        self.assertEqual(game_uses.analyse(self.index, src, start, end, fn, ["self"]).fields["mobs"]["read"], 1)

    def test_self_and_game_paths_are_calls_to_game_methods(self):
        uses = self.uses("via_self")
        self.assertEqual(uses.methods["reset"]["count"], 2)
        self.assertEqual(set(uses.assoc), {"LIMIT"})  # a constant is not in the call graph
        levels = game_uses.transitive(self.index, uses, 1, ())
        callee = levels[0][1][0]
        self.assertEqual(callee["method"], "reset")
        self.assertEqual(callee["uses"].fields["mobs"]["write"], 1)

    def test_repeated_markers_on_one_line_keep_source_order(self):
        markers = self.uses("draws").markers
        draws = [m for m in markers if m["match"] == "next_float"]
        self.assertEqual(len(draws), 2)
        order = [(m["category"], m["match"]) for m in markers if m["category"] in ("send", "entity id")]
        self.assertEqual(order, [("send", "send_packet("), ("entity id", "next_entity")])
        self.assertEqual([m["offset"] for m in markers], sorted(m["offset"] for m in markers))

    def test_a_copy_is_not_a_write_and_setters_are(self):
        fields = self.uses("copies").fields
        self.assertEqual(fields["mobs"]["write"], 1)  # only `as_mut().clear()`
        self.assertEqual(fields["mobs"]["read"], 1)  # `clone().clear()`
        self.assertEqual(fields["level"]["write"], 1)  # `set_difficulty`
        self.assertIn("crate::game::helper", self.uses("copies").names)

    def test_return_arrow_in_generics_and_trait_methods_by_file(self):
        self.assertTrue(self.candidate("apply")[0])
        (_, _, _, _, fn), = self.index.find("game.rs::on_trait")
        self.assertEqual(fn.name, "on_trait")

    def test_bad_line_ranges_are_refused(self):
        for spec in ("game.rs:0-1", "game.rs:9-4", "game.rs:5000-5001"):
            with self.assertRaises(SystemExit):
                self.index.find(spec)


class ResolutionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        root = Path(cls.tmp.name).resolve()
        (root / "game.rs").write_text("struct Game { mobs: u32 }\nimpl Game { fn a(&self) {} }\n")
        (root / "scope.rs").write_text(
            "use horizon_core::geometry::Vec3;\n"
            "impl Game {\n"
            "    fn early(&self) { Vec3::new(); }\n"
            "    fn later(&self) { use horizon_world::Vec3; Vec3::new(); }\n"
            "}\n")
        (root / "other.rs").write_text(
            "use horizon_items::Vec3;\nimpl Game { fn third(&self) { Vec3::new(); } }\n")
        (root / "glob.rs").write_text(
            "use horizon_core::geometry::*;\nimpl Game { fn globbed(&self) { Vec3::new(); } }\n")
        cls.index = game_uses.Index(root, root)
        cls.resolver = game_uses.Resolver(cls.index)
        cls.root = root

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def origin(self, file, name):
        (_, src, start, end, fn), = self.index.find(f"{file}::{name}")
        uses = game_uses.analyse(self.index, src, start, end, fn)
        entry = game_uses.target_report(self.index, self.resolver, name, src, fn, uses)
        return {origin: [r["path"] for r in rows] for origin, rows in entry["names"].items()}

    def test_an_import_in_one_function_does_not_leak_into_another(self):
        self.assertEqual(self.origin("scope.rs", "early"), {"horizon-core": ["Vec3"]})
        self.assertEqual(self.origin("scope.rs", "later"), {"horizon-world": ["Vec3"]})

    def test_workspace_glob_import_resolves_through_the_item_index(self):
        self.assertEqual(self.origin("glob.rs", "globbed"), {"horizon-core": ["Vec3"]})

    def test_combined_report_keeps_each_targets_crate(self):
        combined = game_uses.Uses()
        for file, name in (("scope.rs", "early"), ("other.rs", "third")):
            (_, src, start, end, fn), = self.index.find(f"{file}::{name}")
            game_uses.merge(combined, game_uses.analyse(self.index, src, start, end, fn))
        entry = game_uses.target_report(self.index, self.resolver, "combined", None, None, combined)
        self.assertEqual({o: [r["count"] for r in rows] for o, rows in entry["names"].items()},
                         {"horizon-core": [1], "horizon-items": [1]})

    def test_use_aliases_survive_whitespace_removal(self):
        self.assertEqual(game_uses.expand_use("horizon_command as command"), [("horizon_command", "command")])
        self.assertEqual(sorted(game_uses.expand_use("a::{b as c,\n  d, e::f as g}")),
                         [("a::b", "c"), ("a::d", None), ("a::e::f", "g")])


class BlankTest(unittest.TestCase):
    def test_keeps_offsets_and_lifetimes(self):
        text = "fn f<'a>(x: &'a str) { let c = '}'; /* { */ let s = r#\"}\"#; // }\n}\n"
        out = game_uses.blank(text)
        self.assertEqual(len(out), len(text))
        self.assertEqual(out.count("{"), 1)
        self.assertEqual(out.count("}"), 1)
        self.assertIn("'a", out)


if __name__ == "__main__":
    unittest.main()
