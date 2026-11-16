"""Synthetic trace comparisons: independent observations, no game resources required."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from unittest.mock import patch

# Appended, not prepended: tools/ has its own test_*.py, and unittest discovery must keep finding tests/' copies.
with patch.object(sys, 'path', [*sys.path, str(Path(__file__).resolve().parents[1] / 'tools')]):
    import paper_parity as parity
    from check_public_tree import prohibited


class PaperParityTests(unittest.TestCase):
    def test_plugin_is_public_but_not_its_binaries_or_neighboring_sources(self):
        prefix = 'tools/fixtures/paper-trace-plugin/'
        self.assertFalse(prohibited(prefix + 'PaperTrace.java'))
        for name in ('Other.java', 'PaperTrace.class', 'paper.jar'):
            self.assertTrue(prohibited(prefix + name))

    def test_trace_window_and_normalization(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[1]) as tmp:
            out = Path(tmp)
            def row(name, fields=None):
                return dict(tick=17, name=name, fields=fields or {}, cancelled=False)
            def write(side, rows):
                (out / f'{side}.jsonl').write_text('\n'.join(map(json.dumps, rows)))
            paper = row('ChunkLoadEvent', dict(world='minecraft:overworld', x=-2, z=4, new=True))
            horizon = row('chunk_load', dict(dimension='minecraft:overworld', chunk=[-2, 4], new=True))
            before = row('ChunkLoadEvent', dict(world='minecraft:overworld', x=99, z=99, new=True))
            warmup = row('chunk_load', dict(dimension='minecraft:overworld', chunk=[99, 99], new=True))
            after = row('ChunkLoadEvent', dict(world='minecraft:overworld', x=100, z=100, new=True))
            write('paper', [before, row('_join'), paper, paper, row('_quit'), after])
            write('horizon', [row('_join'), warmup, horizon, row('_quit')])
            parity.lab.write_json(out / 'run.json', dict(complete=True, scenario={'hooks': ['chunk_load']}))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(parity.report(out), 0)
                horizon['fields']['new'] = False
                write('horizon', [row('_join'), warmup, horizon, row('_quit')])
                self.assertEqual(parity.report(out), 1)
                write('horizon', [row('_join'), row('_quit')])
                self.assertEqual(parity.report(out), 1)
                write('horizon', [horizon])
                self.assertEqual(parity.report(out), 1)

    def contract_fixture(self, tmp, hook, cancel=False):
        out = Path(tmp)
        scenario = json.loads(parity.SCENARIOS.read_text())['hotbar' if hook.startswith('player_') else 'moisture']
        manifest = dict(scenario=scenario, cancel=scenario['cancel'] if cancel else {},
                        final_checks={side: ['minecraft:dirt', 'minecraft:grass_block',
                                      'minecraft:farmland[moisture=7]' if cancel else 'minecraft:dirt',
                                      'minecraft:farmland[moisture=7]', 'minecraft:farmland[moisture=7]']
                                      for side in ('paper', 'horizon')})
        def row(name, fields=None, tick=100, cancelled=False):
            return dict(name=name, fields=fields or {}, tick=tick, cancelled=cancelled)
        sides = {}
        for side in ('paper', 'horizon'):
            rows = [row('_join')]
            if hook.startswith('player_'):
                changes = [(0, 3, 140, False), (3, 7, 160, cancel)]
                if not cancel:
                    changes.append((7, 3, 180, False))
                for old, new, tick, denied in changes:
                    fields = dict(old=old, new=new) if side == 'paper' else dict(old_slot=old, new_slot=new)
                    rows.append(row('PlayerItemHeldEvent' if side == 'paper' else hook, fields, tick, denied))
                    if side == 'horizon' and not denied:
                        rows.append(row('player_switch_hotbar_slot_post', fields, tick))
                    rows.append(row('_slot', {'slot': old if denied else new}, tick+1))
            else:
                rows.append(row('_window_start', tick=140))
                for pos in scenario['positions']:
                    denied = cancel and pos == [4, -60, 2]
                    pairs = [(7, 6)] if denied else ([(0, 7)] if pos[2] == 10 else [(7,6),(6,5),(5,4),(4,3),(3,2),(2,1),(1,0)])
                    for a, b in pairs:
                        r = row('MoistureChangeEvent' if side == 'paper' else hook,
                                {'pos': pos, 'from': a, 'to': b}, 150, denied)
                        rows.extend([r] * (3 if denied and side == 'paper' else 1))
                rows.append(row('_window_end', tick=540))
            sides[side] = rows + [row('_quit', tick=600)]
        events = parity.hook_map()[hook]
        def compare():
            for side, rows in sides.items():
                (out / f'{side}.jsonl').write_text('\n'.join(map(json.dumps, rows)))
            with patch.object(parity, 'final_blocks', side_effect=lambda _, side, positions: manifest['final_checks'][side]):
                return parity.compare_contract(out, manifest, hook, events)
        return manifest, sides, compare

    def test_hotbar_sequence_cancel_state_post_and_tick_tolerance(self):
        for cancel in (False, True):
            with tempfile.TemporaryDirectory() as tmp:
                _, sides, compare = self.contract_fixture(tmp, 'player_switch_hotbar_slot_pre', cancel)
                compare()
                held = [r for r in sides['paper'] if r['name'] == 'PlayerItemHeldEvent']
                held[1]['tick'] += 1
                compare()
                held[1]['tick'] += 1
                with self.assertRaisesRegex(ValueError, 'tolerance'): compare()
                held[1]['tick'] -= 2
                if cancel:
                    sides['horizon'].insert(-1, dict(name='player_switch_hotbar_slot_post', tick=160,
                                                    fields={'old_slot':3, 'new_slot':7}, cancelled=False))
                    with self.assertRaisesRegex(ValueError, 'post'): compare()
                    sides['horizon'].pop(-2)
                next(r for r in sides['horizon'] if r['name'] == '_slot')['fields']['slot'] = 7
                with self.assertRaisesRegex(ValueError, 'slot'): compare()

    def test_moisture_ignores_repeated_attempts_but_requires_transitions_and_final_state(self):
        for cancel in (False, True):
            with tempfile.TemporaryDirectory() as tmp:
                manifest, sides, compare = self.contract_fixture(tmp, 'moisture_change', cancel)
                compare()
                manifest['final_checks']['paper'][0] = 'minecraft:farmland[moisture=0]'
                with self.assertRaisesRegex(ValueError, r'paper: final block.*expected.*dirt.*got.*moisture=0'): compare()
                manifest['final_checks']['paper'][0] = 'minecraft:dirt'
                next(r for r in sides['horizon'] if r['name'] == 'moisture_change')['fields']['to'] = 4
                with self.assertRaisesRegex(ValueError, 'transitions'): compare()

    def test_saved_palette_final_states_and_missing_hotbar_row(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            region = out / 'paper/world/dimensions/minecraft/overworld/region/r.0.0.mca'
            region.parent.mkdir(parents=True)
            payload = parity.zlib.compress(b'fixture')
            region.write_bytes(b'\x00\x00\x02\x01' + bytes(8188)
                               + (len(payload) + 1).to_bytes(4, 'big') + b'\x02' + payload)
            chunk = {'sections': [{'Y': -4, 'block_states': {'palette': [
                {'': 'minecraft:grass_block'}, {'id': 'minecraft:farmland', 'properties': {'moisture': '7'}}],
                'data': [0x10]}}]}
            with patch.object(parity, '_nbt', return_value=chunk):
                self.assertEqual(parity.final_blocks(out, 'paper', [[0, -64, 0], [1, -64, 0]]),
                                 ['minecraft:grass_block', 'minecraft:farmland[moisture=7]'])
            _, sides, compare = self.contract_fixture(tmp, 'player_switch_hotbar_slot_pre')
            sides['horizon'] = [r for r in sides['horizon'] if r['tick'] != 180]
            with self.assertRaisesRegex(ValueError, r'horizon: hotbar.*row 3: expected.*7, 3.*got.*missing'):
                compare()

    def test_combined_report_keeps_every_case_and_propagates_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            runs = []
            for hook in ('player_switch_hotbar_slot_pre', 'moisture_change'):
                for cancel in (False, True):
                    name = f'{hook}-{cancel}'
                    case = out / name
                    case.mkdir()
                    manifest, _, compare = self.contract_fixture(case, hook, cancel)
                    compare()
                    parity.lab.write_json(case / 'run.json', dict(manifest, complete=True))
                    runs.append(name)
            parity.lab.write_json(out / 'run.json', {'runs': runs})
            with contextlib.redirect_stdout(io.StringIO()) as output, patch.object(parity, 'final_blocks',
                    side_effect=lambda path, side, positions: json.loads((path / 'run.json').read_text())['final_checks'][side]):
                self.assertEqual(parity.report(out), 0)
                for name in runs:
                    self.assertIn(name, output.getvalue())
                (out / runs[-1] / 'horizon.jsonl').write_text('')
                self.assertEqual(parity.report(out), 1)
