"""tools/anticheat_e2e.py: reading the flag log per player (no server is started)."""

import sys
import unittest
from unittest.mock import patch

# The e2e adds tools/ to sys.path and imports server_bench, which imports tools' own test_* modules by their bare
# names: neither may stay to shadow the tests/ modules of the same names.
with patch.object(sys, 'path', sys.path.copy()), patch.dict(sys.modules):
    from tools import anticheat_e2e as e2e

# Two cheaters on one server, as in check_ground and check_combat: lines of the format Hub::flag writes.
LOG = '\n'.join([
    '[2026-11-13 10:00:00] NoFall0 failed ground (VL 30.0): claims the ground in the air at 1.0 2.0 3.0, kicked (1 of 2 before a ban)',
    '[2026-11-13 10:00:01] Jesus0 failed ground (VL 12.0): claims the ground on a fluid at 1.0 2.0 3.0, claim dropped',
    '[2026-11-13 10:00:02] Aura0 failed combat (VL 20.0): attacked entities 41 and 57 in one tick, banned',
    '[2026-11-13 10:00:03] NoSwing0 failed combat (VL 20.0): attacked without swinging (1 more attacks than swings in a tick), kicked (1 of 2 before a ban)',
    '[2026-11-13 10:00:04] NoSwing0 failed combat (VL 20.0): attacked without swinging (1 more attacks than swings in a tick), banned',
    '[2026-11-13 10:00:05] Aura0 failed combat (VL 20.0): attacked entities 41 and 57 in one tick, kicked (1 of 2 before a ban)',
])


class PlayerLinesTests(unittest.TestCase):
    def test_a_players_lines_for_a_check(self):
        self.assertEqual(len(e2e.player_lines(LOG, 'NoSwing0', 'combat')), 2)
        self.assertEqual(e2e.player_lines(LOG, 'NoSwing0', 'ground'), [])
        self.assertEqual(len(e2e.player_lines(LOG, 'Jesus0', 'ground')), 1)
        # A name that only starts another one's is not that player.
        self.assertEqual(e2e.player_lines(LOG, 'NoSwing', 'combat'), [])

    def test_another_players_kick_is_not_this_ones(self):
        # Jesus0 was never kicked with its ban count, though NoFall0 was.
        self.assertTrue(e2e.warned_of_ban(LOG, 'NoFall0', 'ground'))
        self.assertFalse(e2e.warned_of_ban(LOG, 'Jesus0', 'ground'))

    def test_the_kick_comes_before_the_ban(self):
        self.assertTrue(e2e.kicked_before_banned(LOG, 'NoSwing0', 'combat'))
        # Aura0 has both lines, but the ban first: that is not the kick that warned of it.
        self.assertFalse(e2e.kicked_before_banned(LOG, 'Aura0', 'combat'))
        self.assertFalse(e2e.kicked_before_banned(LOG, 'Jesus0', 'ground'))


if __name__ == '__main__':
    unittest.main()
