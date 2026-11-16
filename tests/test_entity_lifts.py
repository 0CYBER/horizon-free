"""Entity-list whole-pass lifts go through their observable boundary."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIELDS = (
    "mobs|dropped|orbs|boats|carts|armor_stands|item_frames|paintings|tnts|"
    "falling|rockets|tridents|crystals|creatures\\.projectiles|others|"
    "fangs\\.list|ominous_spawners|breath_clouds"
)
RAW_TAKE = re.compile(
    r"(?:std::)?mem::take\s*\(\s*&mut\s+"
    r"(?:self|game|lists)\.(?:" + FIELDS + r")\s*\)"
    r"|std::mem::take\(&mut self\$\(\.\$list\)\+\)"
    r"|std::mem::take\(\s*projectiles\s*\)"
)


class EntityLifts(unittest.TestCase):
    def test_game_entity_lists_have_no_raw_takes(self):
        source = ROOT / "crates/horizon-client/src"
        violations = []
        for path in [source / "game.rs", *sorted((source / "game").rglob("*.rs"))]:
            # Byte-identical upstream source, excluded from discovery and compiled
            # only by the unsupported upstream-parallel-mobs retention feature.
            if path == source / "game/creatures/mob_islands.rs":
                continue
            text = path.read_text()
            for match in RAW_TAKE.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                violations.append(f"{path.relative_to(ROOT)}:{line}")
        self.assertEqual(violations, [], "use the entity list's lift boundary")

    def test_guard_catches_direct_and_generated_entity_lists(self):
        for text in (
            "std::mem::take(&mut self.mobs)",
            "mem::take(\n &mut game.creatures.projectiles\n)",
            "std::mem::take(&mut self$(.$list)+)",
            "std::mem::take(projectiles)",
        ):
            with self.subTest(text=text):
                self.assertIsNotNone(RAW_TAKE.search(text))
        for text in ("std::mem::take(&mut self.mobs[i].outbox)",
                     "std::mem::take(&mut menu.dropped)"):
            with self.subTest(text=text):
                self.assertIsNone(RAW_TAKE.search(text))
