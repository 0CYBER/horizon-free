"""server_wool_vibration's stepped walk is compared only when every tick gave a valid observation."""
import unittest

from tools import test_server_wool_vibration_parity as wool

CASE = 'walk-stone'


def phase(tick):
    return 'active' if 11 <= tick <= 40 else 'cooldown' if 41 <= tick <= 50 else 'inactive'


def transcript(ticks=wool.WALK_TICKS, replace=None):
    """A client transcript of the stepped walk: per tick the frequency, the active test and the cooldown test,
    `replace` mapping a tick to other replies."""
    lines = [f'Alice step={CASE} chat 2, -60, 3 has the following block data: 0']
    for tick in range(1, ticks + 1):
        replies = (replace or {}).get(tick) or [
            f'chat 2, -60, 3 has the following block data: {1 if tick >= 11 else 0}',
            f'chat Test {"passed" if phase(tick) == "active" else "failed"}',
            f'chat Test {"passed" if phase(tick) == "cooldown" else "failed"}']
        lines += [f'Alice step={CASE}#t{tick:02d} {reply}' for reply in replies]
    return lines


def problems_of(lines):
    problems = {}
    cases = wool.compared(lines, problems)
    return cases, wool.stepped_problems(cases, problems).get(CASE, [])


class SteppedRows(unittest.TestCase):
    def test_a_complete_walk_has_no_problems(self):
        cases, problems = problems_of(transcript())
        self.assertEqual(problems, [])
        self.assertIn('t11 frequency 1 active', cases[CASE])
        self.assertIn('t41 frequency 1 cooldown', cases[CASE])

    def test_a_timeout_shared_by_both_sides_is_no_observation(self):
        timeout = ['chat 2, -60, 3 has the following block data: 1',
                   'error no reply to /execute if block 2 -60 3 minecraft:sculk_sensor[sculk_sensor_phase=active]',
                   'chat Test failed']
        lines = transcript(replace={30: timeout})
        vanilla, vanilla_problems = problems_of(lines)
        horizon, horizon_problems = problems_of(lines)
        self.assertEqual(vanilla[CASE], horizon[CASE])  # equal, so only the validation can catch it
        self.assertEqual(len(vanilla_problems), 1)
        self.assertTrue(vanilla_problems[0].startswith('t30: '), vanilla_problems)

    def test_both_phase_tests_passing_is_contradictory(self):
        both = ['chat 2, -60, 3 has the following block data: 1', 'chat Test passed', 'chat Test passed']
        _, problems = problems_of(transcript(replace={45: both}))
        self.assertEqual(problems, ['t45: both the active and the cooldown phase test passed'])

    def test_a_short_sequence_is_incomplete(self):
        _, problems = problems_of(transcript(ticks=wool.WALK_TICKS - 5))
        self.assertEqual(problems, [f'{wool.WALK_TICKS - 5} stepped rows, not {wool.WALK_TICKS} '
                                    '(missing t56, t57, t58, t59, t60)'])


if __name__ == '__main__':
    unittest.main()
