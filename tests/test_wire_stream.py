"""tools/wire_stream.py cuts a capture's serverbound play stream into client ticks: the comparison of a Horizon
client's packets with vanilla's (tools/test_wire_stream_parity.py) is only as good as this cut."""
from pathlib import Path
import importlib.util
import struct
import unittest

# Loaded by path: tools/ on sys.path would shadow tests/ modules that share a name with a tools/ harness.
_spec = importlib.util.spec_from_file_location(
    'wire_stream', Path(__file__).resolve().parents[1] / 'tools' / 'wire_stream.py')
wire_stream = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(wire_stream)


def varint(v):
    v &= 0xFFFFFFFF
    out = b''
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out += bytes([b | 0x80])
        else:
            return out + bytes([b])


def c2s(name, data=b''):
    return {'conn': 1, 'dir': 'c2s', 'state': 'play', 'name': name, 'data': data.hex()}


def mark():
    text = b'wire-mark'
    return c2s('chat_command', varint(len(text)) + text)


LOGIN = {'conn': 1, 'dir': 's2c', 'state': 'play', 'name': 'login', 'data': struct.pack('>i', 42).hex() + '00'}
TICK_END = c2s('client_tick_end')


class WireStreamTest(unittest.TestCase):
    def test_varint_rejects_overlong_encodings_as_java_does(self):
        # Java's VarInt.read allows at most five 7-bit groups; a sixth byte is malformed.
        self.assertEqual(wire_stream.Reader(b'\x00').varint(), 0)          # canonical zero
        self.assertEqual(wire_stream.Reader(bytes.fromhex('ffffffff07')).varint(), 2147483647)
        for overlong in ('808080808000', 'ffffffff8f00', '8080808080808000'):
            with self.subTest(overlong=overlong):
                with self.assertRaisesRegex(ValueError, 'VarInt too big'):
                    wire_stream.Reader(bytes.fromhex(overlong)).varint()
        # One-to-five-byte encodings still decode to exactly the same values, signed and unsigned.
        for v in (0, 1, 127, 128, 300, 1 << 28, (1 << 31) - 1, -(1 << 31), -1):
            with self.subTest(v=v):
                encoded = varint(v)
                self.assertLessEqual(len(encoded), 5)
                self.assertEqual(wire_stream.Reader(encoded).varint(), v)

    def test_ticks_end_at_tick_end_and_start_at_the_mark(self):
        entries = [LOGIN, c2s('move_player_rot', struct.pack('>ffB', 1, 2, 1)), TICK_END, mark(),
                   c2s('player_input', b'\x41'), c2s('move_player_pos', struct.pack('>dddB', 0.5, -60, 1.5, 1)),
                   TICK_END, c2s('pong', b'\0\0\0\7'), c2s('attack', varint(7)), c2s('punch'), TICK_END]
        [run] = wire_stream.scenarios(entries)
        self.assertEqual(run[0]['packets'], [
            ('player_input', {'keys': ['forward', 'sprint']}),
            ('move_player_pos', {'x': 0.5, 'y': -60.0, 'z': 1.5, 'on_ground': True, 'horizontal_collision': False}),
            ('client_tick_end', {})])
        # The pong is the frame's, before the tick's packets: never a tick's own, never misplaced.
        self.assertEqual(run[1]['frame'], [('pong', {'raw': '00000007'})])
        self.assertEqual(run[1]['misplaced'], [])
        # Other entities are named by first appearance, the player as `self`.
        self.assertEqual(run[1]['packets'][0], ('attack', {'entity': 'e1'}))

    def test_an_answer_between_two_packets_of_a_tick_is_misplaced(self):
        entries = [LOGIN, mark(), c2s('player_command', varint(42) + varint(1) + varint(0)), c2s('pong', b'\0\0\0\1'),
                   c2s('move_player_status_only', b'\x01'), TICK_END]
        [run] = wire_stream.scenarios(entries)
        self.assertEqual(run[0]['packets'][0], ('player_command', {'entity': 'self', 'action': 'start_sprinting',
                                                                   'data': 0}))
        self.assertEqual(run[0]['misplaced'], ['pong'])

    def test_a_block_position_decodes_signed_as_vanilla_packs_it(self):
        # BlockPos.asLong: (x & 2^26-1) << 38 | (z & 2^26-1) << 12 | (y & 2^12-1).
        def as_long(x, y, z):
            v = (x & 0x3FFFFFF) << 38 | (z & 0x3FFFFFF) << 12 | (y & 0xFFF)
            return struct.pack('>q', v - (1 << 64) if v >= 1 << 63 else v)
        data = varint(0) + as_long(-2, -61, 3) + varint(1) + struct.pack('>fff', 0.5, 1.0, 0.25) + b'\0\0' + varint(7)
        fields = wire_stream.decode('use_item_on', data)
        self.assertEqual((fields['pos'], fields['face'], fields['sequence']), ([-2, -61, 3], 1, 7))
        self.assertEqual(wire_stream.decode('player_action', varint(0) + as_long(29999999, 2047, -30000000)
                                            + b'\1' + varint(3))['pos'], [29999999, 2047, -30000000])

    def test_a_click_s_state_id_compares_as_how_far_it_lags_the_server(self):
        def set_slot(state):
            return {'conn': 1, 'dir': 's2c', 'state': 'play', 'name': 'container_set_slot',
                    'data': (varint(0) + varint(state) + b'\0\x24\0').hex()}

        def click(state):
            return c2s('container_click', varint(0) + varint(state) + b'\0\x24\0' + varint(0) + b'\0\0')
        # Two runs whose servers counted differently: both clicks are up to date, the third is one behind.
        a = wire_stream.scenarios([LOGIN, set_slot(53), mark(), click(53), TICK_END])
        b = wire_stream.scenarios([LOGIN, set_slot(55), mark(), click(55), TICK_END, set_slot(56), click(55),
                                   TICK_END])
        self.assertEqual(a[0][0]['packets'], b[0][0]['packets'])
        self.assertEqual(a[0][0]['packets'][0][1]['stale'], 0)
        self.assertEqual(b[0][1]['packets'][0][1]['stale'], 1)

    def test_a_late_answer_compares_with_vanilla_s_answer_and_the_script_keeps_its_clock(self):
        tick = lambda *names: {'packets': [(n, {}) for n in names], 'frame': [], 'misplaced': []}  # noqa
        # The server's answer (an abort) reaches vanilla in tick 1 and Horizon in tick 2; the script acts in tick 3.
        vanilla = [tick('punch'), tick('abort', 'punch'), tick('punch'), tick('pos'), tick()]
        horizon = [tick('punch'), tick('punch'), tick('abort', 'punch'), tick('pos'), tick()]
        self.assertEqual(wire_stream.compare_answered(vanilla, horizon, 4, 1, 2), [])
        # What the answer brings is compared: a new dig instead of the abort.
        restart = [tick('punch'), tick('punch'), tick('start', 'punch'), tick('pos'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, restart, 4, 1, 2)], [1])
        # So is the script's clock: a packet a tick late anywhere else.
        late = [tick('punch'), tick('punch'), tick('abort', 'punch'), tick(), tick('pos')]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, late, 4, 1, 2)], [3, 4])

    def test_a_late_answer_on_the_position_reminder_s_tick_keeps_the_reminder_on_the_script_s_clock(self):
        tick = lambda *names: {'packets': [(n, {}) for n in names], 'frame': [], 'misplaced': []}  # noqa
        # #14807: the reminder (sendPosition every 20 ticks) falls in tick 2 for both clients; the answer reaches
        # vanilla in tick 1 and Horizon in tick 2, the reminder's tick.
        vanilla = [tick('punch'), tick('abort', 'punch'), tick('move_player_pos'), tick()]
        horizon = [tick('punch'), tick('punch'), tick('abort', 'punch', 'move_player_pos'), tick()]
        self.assertEqual(wire_stream.compare_answered(vanilla, horizon, 3, 1, 2), [])
        # A reminder a tick early is still a difference, in either answer tick.
        early = [tick('punch'), tick('punch', 'move_player_pos'), tick('abort', 'punch'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, early, 3, 1, 2)], [1, 2])
        # So is a move the answer brings that vanilla's doesn't, outside the reminder's tick.
        moved = [tick('punch'), tick('punch'), tick('abort', 'punch', 'move_player_pos'), tick('move_player_pos')]
        self.assertEqual([d[0] for d in wire_stream.compare_answered(vanilla, moved, 3, 1, 2)], [3])

    def test_a_phase_compares_from_each_client_s_answer_up_to_the_script_s_next_step(self):
        tick = lambda *names: {'packets': [(n, {}) for n in names], 'frame': [], 'misplaced': []}  # noqa
        # The teleport reaches vanilla in tick 1 and Horizon in tick 2; the script's next step is tick 4.
        vanilla = [tick('a'), tick('t0'), tick('t1'), tick('t2'), tick('step'), tick()]
        horizon = [tick('a'), tick('a'), tick('t0'), tick('t1'), tick('step'), tick()]
        self.assertEqual(wire_stream.compare_phased(vanilla, horizon, 5, 1, 2, 4), [])
        # On the script's clock the same streams differ from the answer on.
        self.assertTrue(wire_stream.compare_answered(vanilla, horizon, 5, 1, 2))
        # A difference in the phase is reported at vanilla's tick; one before the earlier answer at its own.
        moved = [tick('a'), tick('a'), tick('t0'), tick('x'), tick('step'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_phased(vanilla, moved, 5, 1, 2, 4)], [2])
        early = [tick('b'), tick('a'), tick('t0'), tick('t1'), tick('step'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_phased(vanilla, early, 5, 1, 2, 4)], [0])
        # With no step after the teleport the phase runs to the script's last tick, never past it on either side.
        ends = [tick('a'), tick('a'), tick('t0'), tick('t1'), tick('t2'), tick('after')]
        self.assertEqual(wire_stream.compare_phased(vanilla[:4] + [tick('t3')], ends, 4, 1, 2, 5), [])

    def test_a_phase_answered_in_the_same_tick_compares_to_the_scenario_s_end(self):
        tick = lambda *names: {'packets': [(n, {}) for n in names], 'frame': [], 'misplaced': []}  # noqa
        # Both clients applied the teleport in tick 1; the script's next step (a release) is tick 3.
        vanilla = [tick('a'), tick('t0'), tick('t1'), tick('release'), tick('stop'), tick()]
        self.assertEqual(wire_stream.compare_phased(vanilla, list(vanilla), 5, 1, 1, 3), [])
        # A missing release and the moves it should have stopped are caught past the step.
        held = [tick('a'), tick('t0'), tick('t1'), tick('t2'), tick('t3'), tick()]
        self.assertEqual([d[0] for d in wire_stream.compare_phased(vanilla, held, 5, 1, 1, 3)], [3, 4])
        # Answered in different ticks, the phase still stops at the step.
        late = [tick('a'), tick('a'), tick('t0'), tick('t1'), tick('t2'), tick('t3')]
        self.assertEqual(wire_stream.compare_phased(vanilla, late, 5, 1, 2, 3), [])

    def test_a_fall_needs_a_move_in_the_air_then_one_on_the_ground(self):
        tick = lambda *grounds: {'packets': [('move_player_pos', {'on_ground': g}) for g in grounds],  # noqa
                                 'frame': [], 'misplaced': []}
        run = [tick(True), tick(False), tick(False), tick(True), tick(True)]
        self.assertTrue(wire_stream.fell_and_landed(run, 1, 4))
        # One move in the air on the last tick: no landing.
        self.assertFalse(wire_stream.fell_and_landed(run[:2], 1, 1))
        # On the ground before the window, landed again inside it without leaving it: no fall.
        self.assertFalse(wire_stream.fell_and_landed(run, 3, 4))
        # A landing after the window's end doesn't count.
        self.assertFalse(wire_stream.fell_and_landed(run, 1, 2))
        # Packets with no flag are skipped.
        run[2]['packets'].insert(0, ('swing', {}))
        self.assertTrue(wire_stream.fell_and_landed(run, 1, 3))

    def test_compare_reports_the_first_field_that_differs(self):
        tick = lambda y: {'packets': [('move_player_pos', {'x': 0.5, 'y': y})], 'frame': [], 'misplaced': []}  # noqa
        diffs = wire_stream.compare([tick(-60.0)], [tick(-60.0000001)], 0)
        self.assertEqual([d[0] for d in diffs], [0])
        self.assertEqual(wire_stream.field_diff(diffs[0][1], diffs[0][2]),
                         ['    #0 move_player_pos.y: vanilla -60.0, Horizon -60.0000001'])
        self.assertEqual(wire_stream.compare([tick(-60.0)], [tick(-60.0)], 0), [])

    def test_field_diff_explains_packets_present_on_only_one_side(self):
        # compare() flags a longer packet list; the explanation must name the extra packet, not be empty.
        v = [('move_player_pos', {'x': 1.0}), ('client_tick_end', {})]
        h = v + [('chat_command', {'command': 'hi'})]
        self.assertEqual([d[0] for d in wire_stream.compare([{'packets': v}], [{'packets': h}], 0)], [0])
        self.assertEqual(wire_stream.field_diff(v, h),
                         ["    #2: vanilla (no packet), Horizon chat_command {'command': 'hi'}"])
        # The mirror case: the extra packet is on the vanilla side.
        self.assertEqual(wire_stream.field_diff(h, v),
                         ["    #2: vanilla chat_command {'command': 'hi'}, Horizon (no packet)"])
        # A whole missing tick (None, as compare() can yield) still explains the surviving side's packets.
        self.assertEqual(wire_stream.field_diff(None, v),
                         ["    #0: vanilla (no packet), Horizon move_player_pos {'x': 1.0}",
                          "    #1: vanilla (no packet), Horizon client_tick_end {}"])


if __name__ == '__main__':
    unittest.main()
