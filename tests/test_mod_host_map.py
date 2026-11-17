import unittest

from tools.layering import mod_host_map


class AreaTests(unittest.TestCase):
    def test_game_file_directly_in_game_dir(self):
        self.assertEqual(mod_host_map.area("crates/horizon-client/src/game/foo.rs"), "game")

    def test_game_file_in_subdirectory(self):
        self.assertEqual(mod_host_map.area("crates/horizon-client/src/game/mobs/ai.rs"), "game/mobs")

    def test_client_file_outside_game(self):
        self.assertEqual(mod_host_map.area("crates/horizon-client/src/net.rs"), "client (non-game)")


if __name__ == "__main__":
    unittest.main()
