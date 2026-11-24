import contextlib
import json
import random
import shutil
import tempfile
import unittest
from pathlib import Path

from tools import parity_fuzz as fuzz
from tools import parity_fuzz_scenarios as families


@contextlib.contextmanager
def stand_in(stub=''):
    """The slices runner with a stand-in fuzzer and a checkout of its own: (the module, a new folder). Its slices'
    work folders are in that folder, never in this checkout's .local/."""
    from tools import parity_fuzz_probe as probe
    real = probe.ROOT, probe.FUZZER
    with tempfile.TemporaryDirectory() as folder:
        probe.ROOT, probe.FUZZER = Path(folder) / 'checkout', Path(folder) / 'stub.py'
        probe.FUZZER.write_text(stub, encoding='utf-8')
        try:
            yield probe, Path(folder)
        finally:
            probe.ROOT, probe.FUZZER = real


FOUND = '{"scenarios": 1, "invalid": [], "refused": [], "groups": []}'


def record(text):
    return {'cmd': '/data get entity @s', 'messages': [[[text, None]]], 'results': [[True, 1]]}


class DispenserSyncTests(unittest.TestCase):
    def test_a_redstone_dispenser_fire_syncs_the_engines_randoms_again(self):
        scenarios = [families.FAMILIES['spawn'](random.Random(seed), seed) for seed in range(200)]

        def fires_a_dispenser(scenario):
            steps = scenario['steps']
            return (any(str(step.get('block', '')).startswith(('dispenser', 'dropper')) for step in steps)
                    and any(step.get('block') == 'redstone_block' for step in steps))

        pulse = next(s for s in scenarios if fires_a_dispenser(s))
        steps = pulse['steps']
        before_fire = next(i for i, step in enumerate(steps) if step['k'] == 'read_near')
        self.assertEqual(steps[before_fire + 1], {'k': 'sync'})  # right before the tick that fires the dispenser
        self.assertNotIn(('drop', before_fire + 1), list(fuzz.edits(pulse)))  # the sync is not shrunk away
        ops = fuzz.render(pulse, 0)
        syncs = [i for i, op in enumerate(ops) if op['op'] == 'sync_randoms']
        self.assertEqual(len(syncs), 2)  # the scenario's own, then the one at the fire
        self.assertEqual(ops[syncs[0]], ops[syncs[1]])  # the same state, drawn from again at the fire
        self.assertEqual(ops[syncs[1] + 1]['op'], 'tick')  # the firing tick is the next op
        quiet = next(s for s in scenarios if not fires_a_dispenser(s))
        self.assertEqual(sum(op['op'] == 'sync_randoms' for op in fuzz.render(quiet, 0)), 1)


class KnownTests(unittest.TestCase):
    def test_known_signatures_are_listed_under_their_issue_and_only_new_ones_are_new(self):
        old = fuzz.KNOWN
        try:
            fuzz.KNOWN = [(r'persist\|trident\|live\|damage', 14823),
                          (r'persist\|pig\|reloaded\|Fire', 99999)]
            groups = {'persist|trident|live|damage': [('a', {})],
                      'persist|pig|live|Air': [('b', {})]}
            known, new, not_observed = fuzz.classify(groups)
            self.assertEqual(known, {14823: ['persist|trident|live|damage']})
            self.assertEqual(new, {'persist|pig|live|Air': [('b', {})]})
            self.assertEqual(not_observed, [(99999, r'persist\|pig\|reloaded\|Fire')])
        finally:
            fuzz.KNOWN = old

    def test_two_patterns_for_one_issue_are_each_tracked(self):
        old = fuzz.KNOWN
        try:
            fuzz.KNOWN = [(r'one', 123), (r'two', 123)]
            known, new, not_observed = fuzz.classify({'one': [('a', {})]})
            self.assertEqual(known, {123: ['one']})
            self.assertEqual(new, {})
            self.assertEqual(not_observed, [(123, 'two')])
        finally:
            fuzz.KNOWN = old


class SnbtDiffTests(unittest.TestCase):
    def test_a_difference_names_its_field_and_key_order_is_not_one(self):
        vanilla = record('Pig has the following entity data: {Motion: [0.0d, 0.1d, 0.0d], Tags: ["a b"], '
                         'UUID: [I; 1, 2, 3, 4], Brain: {memories: {}}, OnGround: 1b}')
        horizon = record('Pig has the following entity data: {OnGround: 0b, Brain: {memories: {}}, '
                         'UUID: [I; 1, 2, 3, 4], Tags: ["a b"], Motion: [0.0d, 0.1d, 0.0d]}')
        self.assertEqual(fuzz.compare(vanilla, vanilla), [])
        self.assertEqual(fuzz.compare(vanilla, horizon), [('OnGround', '1b', '0b')])

    def test_list_lengths_names_and_missing_data_differ(self):
        self.assertEqual(fuzz.diff({'Items': ['1', '2']}, {'Items': ['1']}), [('Items[#]', 2, 1)])
        self.assertEqual(fuzz.diff({'a': {'b': '1s'}}, {'a': {}}), [('a.b', '1s', None)])
        renamed = record('Arrow has the following entity data: {}')
        self.assertEqual(fuzz.compare(record('Spectral Arrow has the following entity data: {}'), renamed),
                         [('<name>', 'Spectral Arrow', 'Arrow')])
        missing = {'cmd': '/data get entity @s', 'messages': [[['No entity was found', None]]], 'results': [[False, 0]]}
        self.assertEqual(fuzz.compare(renamed, missing)[0][0], '<result>')
        self.assertEqual(fuzz.normalise('Items[3].components[0].id'), 'Items[].components[].id')

    def test_absolute_game_times_compare_only_whether_they_are_set(self):
        vanilla = record('Villager has the following entity data: {LastGossipDecay: 6612L}')
        self.assertEqual(fuzz.compare(vanilla, record('Villager has the following entity data: {LastGossipDecay: 1294L}')), [])
        self.assertEqual(fuzz.compare(vanilla, record('Villager has the following entity data: {LastGossipDecay: 0L}')),
                         [('LastGossipDecay', '6612L', '0L')])
        self.assertEqual(fuzz.compare(vanilla, record('Villager has the following entity data: {}')),
                         [('LastGossipDecay', '6612L', None)])

    def test_attributes_compare_in_any_order_but_by_value(self):
        a = {'attributes': [{'id': 'b', 'base': '1.0d'}, {'id': 'a', 'base': '2.0d'}]}
        self.assertEqual(fuzz.diff(a, {'attributes': a['attributes'][::-1]}), [])
        changed = {'attributes': [{'id': 'a', 'base': '2.0d'}, {'id': 'b', 'base': '3.0d'}]}
        self.assertEqual(fuzz.diff(a, changed), [('attributes[1].base', '1.0d', '3.0d')])

    def test_tags_compare_as_a_set(self):
        self.assertEqual(fuzz.diff({'Tags': ['"fz"', '"fz7"']}, {'Tags': ['"fz7"', '"fz"']}), [])
        self.assertEqual(fuzz.diff({'Passengers': [{'Tags': ['"b"', '"a"']}]}, {'Passengers': [{'Tags': ['"a"', '"c"']}]}),
                         [('Passengers[0].Tags[1]', '"b"', '"c"')])


class ShrinkTests(unittest.TestCase):
    scenario = {'family': 'persist', 'seed': 1, 'steps': [
        {'k': 'summon', 'type': 'pig', 'at': [7.5, 0, 7.5], 'n': 0, 'nbt': {'NoAI': '1b', 'Fire': '20s'}},
        {'k': 'tick', 'n': 4}, {'k': 'read_entity', 'n': 0}]}

    def test_edits_drop_steps_and_keys_and_halve_ticks(self):
        edits = list(fuzz.edits(self.scenario))
        self.assertIn(('nbt', 0, 'Fire'), edits)
        self.assertIn(('half', 1), edits)
        reduced = fuzz.reduce(self.scenario, {('nbt', 0, 'Fire'), ('half', 1)})
        self.assertEqual(reduced['steps'][0]['nbt'], {'NoAI': '1b'})
        self.assertEqual(reduced['steps'][1]['n'], 2)
        self.assertEqual(len(fuzz.reduce(self.scenario, {('drop', 1)})['steps']), 2)
        self.assertEqual(self.scenario['steps'][0]['nbt'], {'NoAI': '1b', 'Fire': '20s'})

    def test_each_read_divergence_counts_not_only_the_first(self):
        result = {'invalid': None, 'failures': [('persist|pig|live|Air', '/a', []), ('persist|pig|live|Air', '/b', []),
                                                ('persist|pig|reloaded|Fire', '/c', [])]}
        self.assertEqual(fuzz.signatures(result), ['persist|pig|live|Air', 'persist|pig|reloaded|Fire'])
        self.assertEqual(fuzz.signatures({**result, 'invalid': 'kept loaded'}), [])
        self.assertEqual(fuzz.signatures({'crash': 'log'}), ['crash'])

    def test_rendered_scenarios_sync_randoms_and_give_their_entity_a_fixed_uuid(self):
        ops = fuzz.render(self.scenario, 2)
        self.assertEqual([op['op'] for op in ops[:4]], ['cmd', 'wait_chunks', 'cmd', 'sync_randoms'])
        summon = next(op['cmd'] for op in ops if op['op'] == 'cmd' and op['cmd'].startswith('/summon'))
        self.assertIn('UUID:[I;1179998810,2,0,1]', summon)
        read = next(op for op in ops if op.get('read'))
        self.assertEqual(read['cmd'], '/data get entity 46555a5a-0000-0002-0000-000000000001')


class FamilyTests(unittest.TestCase):
    def test_a_seed_gives_one_runnable_scenario(self):
        families.KNOWN.update({'entity': {'pig': {'PersistenceRequired': '0b', 'Air': '300s', 'Motion': ['0.0d'] * 3},
                                          'arrow': {'life': '0s', 'crit': '0b'}},
                               'block': {'chest': {'Items': []}, 'furnace': {'cooking_time_spent': '0s'}}})
        for name, family in families.FAMILIES.items():
            for seed in range(200):
                scenario = family(random.Random(seed), seed)
                self.assertEqual(scenario, family(random.Random(seed), seed))
                self.assertTrue(fuzz.runnable(scenario), (name, seed))
                self.assertTrue(any(op.get('read') for op in fuzz.render(scenario, 1)), (name, seed))


class StackTests(unittest.TestCase):
    step = {'k': 'stack', 'how': 'with', 'at': [7, 2, 7], 'item': 'minecraft:iron_pickaxe', 'count': 1,
            'nbt': {'minecraft:damage': '0', 'minecraft:lore': None}}

    def test_a_stack_renders_as_a_command_argument_and_as_saved_nbt(self):
        self.assertEqual(fuzz.stack_command(self.step, '1 2 3'), '/item replace block 1 2 3 container.0 with '
                         'minecraft:iron_pickaxe[minecraft:damage=0,!minecraft:lore] 1')
        saved = '{Slot:0b,id:"minecraft:iron_pickaxe",count:1,components:{"minecraft:damage":0,"!minecraft:lore":{}}}'
        self.assertEqual(fuzz.stack_command({**self.step, 'how': 'nbt', 'block': 'barrel'}, '1 2 3'),
                         f'/setblock 1 2 3 barrel{{Items:[{saved}]}}')
        self.assertEqual(fuzz.stack_command({**self.step, 'how': 'merge'}, '1 2 3'),
                         f'/data merge block 1 2 3 {{Items:[{saved}]}}')
        self.assertIn('{type:"minecraft:set_components",components:{"minecraft:damage":0,"!minecraft:lore":{}}}',
                      fuzz.stack_command({**self.step, 'how': 'loot'}, '1 2 3'))
        self.assertEqual(fuzz.stack_command({**self.step, 'nbt': {}}, '1 2 3'),
                         '/item replace block 1 2 3 container.0 with minecraft:iron_pickaxe 1')

    def test_a_patch_entry_shrinks_away_and_a_command_keeps_its_own_braces(self):
        scenario = {'family': 'items', 'seed': 1, 'steps': [
            self.step, {'k': 'cmd', 'cmd': '/execute if items block {0} container.0 *[minecraft:custom_data~{a:1b}]',
                        'at': [[7, 2, 7]], 'subject': 'if_items/partial'},
            {'k': 'read_block', 'at': [7, 2, 7], 'path': 'Items', 'subject': 'with'}]}
        self.assertIn(('nbt', 0, 'minecraft:lore'), list(fuzz.edits(scenario)))
        commands = [op['cmd'] for op in fuzz.render(scenario, 0) if op.get('step', -1) >= 0]
        self.assertEqual(commands[1], '/execute if items block 4103 302 4103 container.0 *[minecraft:custom_data~{a:1b}]')
        self.assertEqual(commands[2], '/data get block 4103 302 4103 Items')
        self.assertEqual(fuzz.subject(scenario, scenario['steps'][2]), 'with')

    def test_a_merge_goes_into_the_block_already_placed(self):
        scenario = {'family': 'blockdata', 'seed': 1, 'steps': [
            {'k': 'block', 'block': 'chest', 'at': [7, 0, 7]},
            {'k': 'block', 'block': 'chest', 'at': [7, 0, 7], 'nbt': {'CustomName': '"fz"'}, 'merge': True},
            {'k': 'read_block', 'at': [7, 0, 7]}]}
        commands = [op['cmd'] for op in fuzz.render(scenario, 0) if op.get('step', -1) >= 0]
        self.assertEqual(commands[:2], ['/setblock 4103 300 4103 chest',
                                        '/data merge block 4103 300 4103 {CustomName:"fz"}'])

    def test_prototype_values_become_snbt(self):
        from tools import parity_fuzz_data as data
        self.assertEqual(data.to_snbt({'a': [1, 2.5, True, 'x"y'], 'minecraft:b': {}}),
                         '{"a":[1,2.5d,true,"x\\"y"],"minecraft:b":{}}')
        rng = random.Random(3)
        patched = data.patch(rng, {'minecraft:max_stack_size': 64, 'minecraft:lore': []})
        self.assertTrue(patched and all(value is None or isinstance(value, str) for value in patched.values()))
        self.assertEqual(data.mistype(random.Random(1), '"x"'), '7')
        self.assertNotEqual(data.mistype(random.Random(1), '5L'), '5L')


    def test_ordinary_inputs_keep_to_command_built_stacks_and_values_vanilla_saves(self):
        from tools import parity_fuzz_data as data
        data.EDGES = False
        try:
            for seed in range(60):
                routes = [step['how'] for step in data.items(random.Random(seed), seed)['steps'] if step['k'] == 'stack']
                self.assertTrue(set(routes) <= {'with', 'modify', 'loot'}, routes)
                placed = [step for step in data.blockdata(random.Random(seed), seed)['steps'] if step.get('nbt')]
                for step in placed:
                    block = step['block']
                    self.assertTrue(all(value == data.BLOCKS[block][key][0] for key, value in step['nbt'].items()))
        finally:
            data.EDGES = True

    def test_shard_logs_merge_into_clusters_by_field_and_kind_of_difference(self):
        from tools import parity_fuzz_probe as probe
        where = [7, '/data get block 1 2 3 Items', [['[0].components.minecraft:food.saturation', '8.0d', '8.0f']]]
        self.assertEqual(probe.cluster('items|with|live|[0].components', where),
                         'items|with|[].components.minecraft:food.saturation|tag type')
        where[2][0][1:] = [None, '{}']
        self.assertTrue(probe.cluster('items|with|live|x', where).endswith('|only horizon'))


    def test_a_stack_count_stays_within_the_limit_its_patch_leaves(self):
        from tools import parity_fuzz_data as data
        slab = {'minecraft:max_stack_size': 64}
        self.assertEqual(data.stack_limit(slab, {'minecraft:max_stack_size': '1'}), (1, True))
        self.assertEqual(data.stack_limit(slab, {'minecraft:max_stack_size': None}), (1, True))
        self.assertEqual(data.stack_limit(slab, {'minecraft:max_damage': '250'}), (64, False))
        self.assertEqual(data.stack_limit({'minecraft:max_stack_size': 1, 'minecraft:max_damage': 250}, {}), (1, True))
        old = dict(data.PROTOTYPES), dict(data.GAME)
        data.PROTOTYPES.update({'minecraft:spruce_slab': slab, 'minecraft:iron_pickaxe': {
            'minecraft:max_stack_size': 1, 'minecraft:max_damage': 250}})
        data.GAME.update({'minecraft:max_stack_size': ['1', '16', '64'], 'minecraft:max_damage': ['250']})
        data.EDGES = False
        try:
            for seed in range(400):
                for step in data.items(random.Random(seed), seed)['steps']:
                    if step['k'] == 'stack':
                        limit, valid = data.stack_limit(data.PROTOTYPES.get(step['item'], {}), step['nbt'])
                        self.assertTrue(valid and step['count'] <= limit, (seed, step))
        finally:
            data.EDGES = True
            for store, kept in zip((data.PROTOTYPES, data.GAME), old):
                store.clear()
                store.update(kept)

    def test_a_retried_shard_counts_once_in_a_merge(self):
        import tempfile
        from tools import parity_fuzz_probe as probe
        found = {'scenarios': 50, 'invalid': [], 'refused': [], 'groups': [
            {'signature': 'items|with|live|x', 'seeds': [3], 'where': [[3, '/data get', [['x', '1', '2']]]]}]}
        with tempfile.TemporaryDirectory() as folder:
            paths = []
            for name in ('fuzz_probe#1.log', 'fuzz_probe#1.first-run.log', 'fuzz_probe#2.first-run.log'):
                paths.append(Path(folder) / name)
                paths[-1].write_text('=== found.json ===' + chr(10) + json.dumps(found) + chr(10), encoding='utf-8')
            merged = probe.merge(paths)
        self.assertEqual(merged['totals']['scenarios'], 100)
        self.assertEqual(merged['groups']['items|with|live|x']['seeds'], [3, 3])

    def test_a_merge_takes_slices_own_found_files_each_named_by_its_folder(self):
        import tempfile
        from tools import parity_fuzz_probe as probe
        found = json.dumps({'scenarios': 25, 'invalid': [], 'refused': [[4, '/x']], 'groups': [
            {'signature': 'commands|cmd/x|live|y', 'seeds': [4], 'where': [[4, '/x', [['y', '1', None]]]]}]})
        with tempfile.TemporaryDirectory() as folder:
            paths = [Path(folder) / f'parity-fuzz-results-{runner}' / f'slice-{first}' / 'found.json'
                     for runner, first in ((1, 1), (1, 26), (2, 201))]
            for path in paths:
                path.parent.mkdir(parents=True)
                path.write_text(found, encoding='utf-8')
            merged = probe.merge(paths)
        self.assertEqual(merged['totals'], {'scenarios': 75, 'invalid': 0, 'refused': 3})
        self.assertEqual(list(merged['clusters']), ['commands|cmd/x|y|only vanilla'])

    def test_slices_run_side_by_side_each_with_its_own_home_and_a_lost_one_fails_the_run(self):
        import argparse
        # A stand-in fuzzer: writes found.json into its work folder, but for seed 30; says which game home it got.
        stub = ('import json, os, sys\n'
                'args = sys.argv[1:]\n'
                'seed, work = args[args.index("--seed") + 1], args[args.index("--work-dir") + 1]\n'
                'print("home", os.path.basename(os.path.dirname(os.environ["HORIZON_HOME"])), "args", " ".join(args[:7]))\n'
                'if seed != "30":\n'
                '    json.dump({"scenarios": int(args[args.index("--count") + 1]), "invalid": [], "refused": [],\n'
                '               "groups": []}, open(os.path.join(work, "found.json"), "w"))\n')
        with stand_in(stub) as (probe, out):
            base = dict(family='commands', seed=10, count=10, slices=3, seeds=None, shrink_rounds=0, ordinary=True,
                        binary=out / 'game', limit=60)
            with self.assertRaises(SystemExit) as lost:
                probe.slices(argparse.Namespace(**base, out=out / 'a'))
            self.assertIn('[30]', str(lost.exception.code))
            with self.assertRaises(SystemExit) as fine:
                probe.slices(argparse.Namespace(**{**base, 'seeds': [7, 9]}, out=out / 'b'))
            self.assertEqual(fine.exception.code, 0)
            self.assertEqual(sorted(path.name for path in (out / 'a').iterdir()), ['slice-10', 'slice-20', 'slice-30'])
            self.assertTrue((out / 'a/slice-20/found.json').exists() and not (out / 'a/slice-30/found.json').exists())
            self.assertIn('home slice-20 args commands --seed 20 --count 10 --batch 25',
                          (out / 'a/slice-20/output.txt').read_text(encoding='utf-8'))
            self.assertIn('--seed 9 --count 1 ', (out / 'b/slice-9/output.txt').read_text(encoding='utf-8'))
            self.assertEqual(probe.merge(sorted(out.glob('*/*/found.json')))['totals']['scenarios'], 22)
            # The slices' work folders are the stand-in checkout's.
            self.assertEqual(sorted(path.name for path in (out / 'checkout/.local/parity-fuzz').iterdir()),
                             ['slice-10', 'slice-20', 'slice-30', 'slice-7', 'slice-9'])

    def test_slices_refuse_a_seed_twice_no_slice_and_a_results_folder_they_must_not_remove(self):
        import argparse
        import os
        with stand_in() as (probe, folder):
            base = dict(family='commands', seed=1, count=1, slices=1, shrink_rounds=0, ordinary=True,
                        binary=folder / 'game', limit=60)
            with self.assertRaises(SystemExit) as twice:
                probe.slices(argparse.Namespace(**base, seeds=[5, 7, 5], out=folder / 'a'))
            self.assertIn('named twice', str(twice.exception.code))
            with self.assertRaises(SystemExit) as none:
                probe.slices(argparse.Namespace(**{**base, 'slices': 0}, seeds=None, out=folder / 'a'))
            self.assertIn('no slice to run', str(none.exception.code))
            # --out is removed with all it holds: not the checkout, a folder above it or the slices' work folders.
            (probe.ROOT / '.local' / 'parity-fuzz').mkdir(parents=True)
            (probe.ROOT / 'kept.txt').write_text('kept', encoding='utf-8')
            here = Path.cwd()
            os.chdir(probe.ROOT)
            try:
                for out in (Path('.'), probe.ROOT, folder, probe.ROOT.parent.parent, probe.ROOT / '.local',
                            Path('.local/parity-fuzz'), probe.ROOT / '.local/parity-fuzz/slice-5'):
                    with self.assertRaises(SystemExit) as refused:
                        probe.slices(argparse.Namespace(**base, seeds=[5], out=out))
                    self.assertIn('would be removed', str(refused.exception.code), out)
            finally:
                os.chdir(here)
            self.assertEqual((probe.ROOT / 'kept.txt').read_text(encoding='utf-8'), 'kept')
            self.assertEqual(list((probe.ROOT / '.local' / 'parity-fuzz').iterdir()), [])  # and nothing started
            # .local as a link to a folder elsewhere: the work folders are then outside the checkout, which is no
            # less the checkout.
            elsewhere = folder / 'elsewhere'
            (elsewhere / 'parity-fuzz').mkdir(parents=True)
            shutil.rmtree(probe.ROOT / '.local')
            try:
                os.symlink(elsewhere, probe.ROOT / '.local', target_is_directory=True)
            except OSError:
                self.skipTest('this account cannot make links')
            os.chdir(probe.ROOT)
            try:
                for out in (Path('.'), Path('..'), probe.ROOT, folder, elsewhere, elsewhere / 'parity-fuzz',
                            Path('.local/parity-fuzz/slice-5')):
                    with self.assertRaises(SystemExit) as refused:
                        probe.slices(argparse.Namespace(**base, seeds=[5], out=out))
                    self.assertIn('would be removed', str(refused.exception.code), out)
            finally:
                os.chdir(here)
            self.assertEqual((probe.ROOT / 'kept.txt').read_text(encoding='utf-8'), 'kept')
            self.assertEqual(list((elsewhere / 'parity-fuzz').iterdir()), [])
            # A folder of its own under the checkout is fine.
            with self.assertRaises(SystemExit):
                probe.slices(argparse.Namespace(**base, seeds=[5], out=probe.ROOT / 'results'))
            self.assertEqual([path.name for path in (probe.ROOT / 'results').iterdir()], ['slice-5'])

    def test_slices_make_the_results_folder_new(self):
        import argparse
        import os
        with stand_in() as (probe, folder):
            base = dict(family='commands', seed=1, count=1, slices=1, shrink_rounds=0, ordinary=True,
                        binary=folder / 'game', limit=60)
            # A results folder someone left, with a file and (where links can be made) a link out of it.
            planted, outside = folder / 'b', folder / 'outside'
            planted.mkdir()
            outside.mkdir()
            (outside / 'kept.txt').write_text('kept', encoding='utf-8')
            (planted / 'planted.txt').write_text('planted', encoding='utf-8')
            try:
                os.symlink(outside, planted / 'link', target_is_directory=True)
            except OSError:
                pass
            with self.assertRaises(SystemExit):
                probe.slices(argparse.Namespace(**base, seeds=[991], out=planted))
            self.assertEqual([path.name for path in planted.iterdir()], ['slice-991'])
            self.assertEqual((outside / 'kept.txt').read_text(encoding='utf-8'), 'kept')
            try:
                shutil.rmtree(planted)
                os.symlink(outside, planted, target_is_directory=True)
            except OSError:
                return
            with self.assertRaises(SystemExit):
                probe.slices(argparse.Namespace(**base, seeds=[991], out=planted))
            self.assertFalse(planted.is_symlink())
            self.assertEqual(sorted(path.name for path in outside.iterdir()), ['kept.txt'])

    def test_slices_say_they_staged_only_once_they_have_and_also_beside_a_lost_slice(self):
        import argparse
        import os
        # A stand-in fuzzer: a result for every seed but 72; says which of the runner's variables it was given.
        stub = ('import os, sys\n'
                'args = sys.argv[1:]\n'
                'seed, work = args[args.index("--seed") + 1], args[args.index("--work-dir") + 1]\n'
                'print("given", sorted(name for name in os.environ if name.startswith("GITHUB_")))\n'
                'if seed != "72":\n'
                '    open(os.path.join(work, "found.json"), "w").write(\'FOUND\')\n').replace('FOUND', FOUND)
        before = dict(os.environ)
        with stand_in(stub) as (probe, folder):
            real_staged = probe.staged
            output, out = folder / 'step-output', folder / 'out'
            output.touch()
            # As on a runner: the five files a step speaks to the runner through, and what only describes the run.
            for name in [name for name in os.environ if name.startswith('GITHUB_')]:
                del os.environ[name]
            os.environ.update({name: str(folder / name.lower()) for name in probe.STEP_FILES},
                              GITHUB_OUTPUT=str(output), GITHUB_ACTIONS='true', GITHUB_REPOSITORY='horizon-rust/horizon')
            self.assertEqual(sorted(probe.STEP_FILES), ['GITHUB_ENV', 'GITHUB_OUTPUT', 'GITHUB_PATH', 'GITHUB_STATE',
                                                        'GITHUB_STEP_SUMMARY'])
            arguments = argparse.Namespace(family='commands', seed=1, count=1, slices=1, seeds=[71, 72], shrink_rounds=0,
                                           ordinary=True, binary=folder / 'game', limit=60, out=out)
            try:
                # It stops while staging, and a file of an allowed name was in the results folder before it began.
                (out / 'slice-71').mkdir(parents=True)
                (out / 'slice-71' / 'found.json').write_text(FOUND, encoding='utf-8')

                def stops(*parts, **more):
                    raise RuntimeError('stopped while staging')
                probe.staged = stops
                with self.assertRaises(RuntimeError):
                    probe.slices(arguments)
                self.assertEqual(output.read_text(encoding='utf-8'), '')
                self.assertEqual(list(out.rglob('*.json')), [])
                # It stages, one slice lost: it fails, and has said what it staged.
                probe.staged = real_staged
                with self.assertRaises(SystemExit) as lost:
                    probe.slices(arguments)
                self.assertIn('[72]', str(lost.exception.code))
            finally:
                probe.staged = real_staged
                os.environ.clear()
                os.environ.update(before)
            self.assertEqual(output.read_text(encoding='utf-8'), f'staged={probe.digest(out)}\n')
            self.assertRegex(probe.digest(out), r'^[0-9a-f]{64}$')
            self.assertEqual(sorted(path.relative_to(out).as_posix() for path in out.rglob('*') if path.is_file()),
                             ['slice-71/found.json', 'slice-71/output.txt', 'slice-72/output.txt'])
            # No slice is given a step file; the rest stays.
            self.assertIn("given ['GITHUB_ACTIONS', 'GITHUB_REPOSITORY']",
                          (out / 'slice-72/output.txt').read_text(encoding='utf-8'))
            (out / 'slice-72/output.txt').write_text('changed', encoding='utf-8')
            self.assertNotIn(probe.digest(out), output.read_text(encoding='utf-8'))

    def test_a_result_with_the_fuzzers_own_lengths_and_witness_results_is_whole_and_counted(self):
        from tools import parity_fuzz_probe as probe
        # What the fuzzer itself gives: two lists' lengths (numbers), a witness's results (lists), a tag's text, a
        # field only one engine has, and whatever else two values of saved data may be.
        lengths = fuzz.diff({'Items': ['a', 'b'], 'Air': '300s', 'Tags': ['x']}, {'Items': ['a'], 'Air': '300'})
        self.assertEqual(lengths, [('Air', '300s', '300'), ('Items[#]', 2, 1), ('Tags', ['x'], None)])
        guard = {'guard': 'shulker_hit', 'cmd': '/damage'}
        _, failure = fuzz.witness('shulker', guard, {'results': [[True, 26]]}, {'results': [[False, 30]]})
        self.assertEqual(failure[2], [('<hit>', [[True, 26]], [[False, 30]])])
        other = fuzz.diff({'a': True, 'b': 1.5, 'c': {'d': 1}}, {'a': False, 'b': 2, 'c': 'text'})
        found = {'scenarios': 4, 'invalid': [], 'refused': [], 'groups': [
            {'signature': 'items|give|live|Air', 'seeds': [3], 'where': [[3, '/give', lengths]]},
            {'signature': 'items|give|live|Items[#]', 'seeds': [4], 'where': [[4, '/give', lengths[1:]]]},
            {'signature': failure[0], 'seeds': [5], 'where': [[5, failure[1], failure[2]]]},
            {'signature': 'items|data|live|a', 'seeds': [6, 7], 'where': [[6, '/data', other], [7, '/data', other[1:]]]}]}
        found = json.loads(json.dumps(found))  # as read back from found.json: tuples are lists
        self.assertTrue(probe.complete(found))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'slice-3' / 'found.json'
            path.parent.mkdir()
            path.write_text(json.dumps(found), encoding='utf-8')
            merged = probe.merge([path])
        self.assertEqual((merged['counted'], merged['lost'], merged['totals']['scenarios']), (1, [], 4))
        self.assertEqual(sorted(merged['clusters']), [
            'items|data|a|value', 'items|data|b|value', 'items|give|Air|tag type', 'items|give|Items[#]|value',
            'shulker|shulker_hit|<hit>|value'])
        self.assertEqual(probe.cluster('items|give|live|Tags', [8, '/give', [['Tags', ['x'], None]]]),
                         'items|give|Tags|only vanilla')

    def test_only_a_regular_file_reached_through_no_link_is_staged(self):
        import os
        import tempfile
        from tools import parity_fuzz_probe as probe
        with tempfile.TemporaryDirectory() as folder:
            work, outside = Path(folder) / 'work', Path(folder) / 'outside'
            (work / 'batch-1').mkdir(parents=True)
            outside.mkdir()
            (work / 'found.json').write_text('{"a": 1}', encoding='utf-8')
            (work / 'batch-1' / 'harness.log').write_text('log', encoding='utf-8')
            (outside / 'harness.log').write_text('not a slice file', encoding='utf-8')
            self.assertEqual(probe.staged(work, 'found.json'), '{"a": 1}')
            self.assertEqual(probe.staged(work, 'batch-1', 'harness.log'), 'log')
            self.assertIsNone(probe.staged(work, 'found.json', limit=3))
            self.assertIsNone(probe.staged(work, 'batch-1'))  # a folder
            self.assertIsNone(probe.staged(work, 'report.json'))  # missing
            try:
                os.link(outside / 'harness.log', work / 'batch-1' / 'second-name.log')
                os.link(work / 'found.json', outside / 'found.json')
            except OSError:
                pass
            else:  # a file with a second name, here or elsewhere
                self.assertIsNone(probe.staged(work, 'batch-1', 'second-name.log'))
                self.assertIsNone(probe.staged(work, 'found.json'))
            try:
                os.symlink(outside / 'harness.log', work / 'report.json')
                os.symlink(outside, work / 'batch-2', target_is_directory=True)
                os.symlink(work, Path(folder) / 'linked', target_is_directory=True)
            except OSError:
                self.skipTest('this account cannot make links')
            self.assertIsNone(probe.staged(work, 'report.json'))  # the file is a link
            self.assertIsNone(probe.staged(work, 'batch-2', 'harness.log'))  # a link on the way
            self.assertIsNone(probe.staged(Path(folder) / 'linked', 'found.json'))  # the slice's folder is one

    def test_a_slices_found_file_is_kept_only_whole_and_a_merge_counts_no_other(self):
        import argparse
        import os
        # A stand-in fuzzer whose found.json is not JSON (seed 61), a list (62), a link to a file elsewhere (63), whole
        # (64), or an object that lacks what a merge counts (65, 66); each leaves a report.json that is an object.
        stub = ('import json, os, sys\n'
                'args = sys.argv[1:]\n'
                'seed, work = args[args.index("--seed") + 1], args[args.index("--work-dir") + 1]\n'
                'found = os.path.join(work, "found.json")\n'
                'open(os.path.join(work, "report.json"), "w").write(json.dumps({"seed": seed}))\n'
                'if seed == "63":\n'
                '    try:\n'
                '        os.symlink(os.environ["ELSEWHERE"], found)\n'
                '    except OSError:\n'
                '        pass\n'
                'else:\n'
                '    open(found, "w").write(json.load(open(os.environ["SHAPES"]))[seed])\n')
        whole = {'scenarios': 3, 'invalid': [], 'refused': [[4, '/x']], 'groups': [
            {'signature': 'commands|cmd/x|live|y', 'seeds': [4], 'where': [[4, '/x', [['y', '1', None]]]]}]}
        partly = {'scenarios': 1, 'invalid': [], 'refused': [], 'groups': [{'signature': 'a', 'seeds': [1], 'where': []}]}
        with stand_in(stub) as (probe, folder):
            elsewhere = folder / 'elsewhere.json'
            elsewhere.write_text(json.dumps(whole), encoding='utf-8')
            (folder / 'shapes.json').write_text(json.dumps({
                '61': '{not json', '62': '[1]', '64': json.dumps(whole), '65': '{"scenarios": 1}',
                '66': json.dumps(partly)}), encoding='utf-8')
            os.environ.update(ELSEWHERE=str(elsewhere), SHAPES=str(folder / 'shapes.json'))
            try:
                with self.assertRaises(SystemExit) as lost:
                    probe.slices(argparse.Namespace(family='commands', seed=1, count=1, slices=1, shrink_rounds=0,
                                                    seeds=[61, 62, 63, 64, 65, 66], ordinary=True,
                                                    binary=folder / 'game', limit=60, out=folder / 'out'))
            finally:
                del os.environ['ELSEWHERE'], os.environ['SHAPES']
            self.assertIn('[61, 62, 63, 65, 66]', str(lost.exception.code))
            out = folder / 'out'
            self.assertEqual(sorted(path.relative_to(out).as_posix() for path in out.rglob('found.json')),
                             ['slice-64/found.json'])
            # A lost slice keeps what says why: its output and its report.
            self.assertEqual(sorted(path.name for path in (out / 'slice-65').iterdir()), ['output.txt', 'report.json'])
            for path in out.rglob('*'):
                self.assertFalse(path.is_symlink(), path)
            # A merge given such files all the same counts the whole one and names the others lost.
            given = []
            for name, text in (('whole', json.dumps(whole)), ('bare', '{"scenarios": 1}'), ('partly', json.dumps(partly)),
                               ('list', '[1]'), ('broken', '{not json')):
                (folder / name).mkdir()
                (folder / name / 'found.json').write_text(text, encoding='utf-8')
                given.append(folder / name / 'found.json')
            merged = probe.merge(given)
            self.assertEqual(merged['totals'], {'scenarios': 3, 'invalid': 0, 'refused': 1})
            self.assertEqual(len(merged['lost']), 4)
            self.assertEqual(merged['counted'], 1)
            # What `cluster` reads of a group is checked too: no such file reaches it.
            place = whole['groups'][0]['where'][0]
            for differences in (7, {'y': 1}, 'y', [7], [['y', '1']], [[7, '1', '2']], [[None, '1', '2']], [{'y': 1}]):
                broken = {**whole, 'groups': [{**whole['groups'][0], 'where': [[*place[:2], differences]]}]}
                self.assertFalse(probe.complete(broken), differences)
                (folder / 'whole' / 'found.json').write_text(json.dumps(broken), encoding='utf-8')
                self.assertEqual(probe.merge(given)['counted'], 0, differences)
            for differences in ([], [['y', None, '1b'], ['z', '1', '2', 'more']]):
                fine = {**whole, 'groups': [{**whole['groups'][0], 'where': [[*place[:2], differences]]}]}
                self.assertTrue(probe.complete(fine), differences)
                self.assertIsInstance(probe.cluster('commands|cmd/x|live|y', fine['groups'][0]['where'][0]), str)
            self.assertTrue(probe.complete({**whole, 'groups': [{**whole['groups'][0], 'where': [place[:2]]}]}))
            # The command says whether as many shards as planned had a whole result, after it has printed the table.
            (folder / 'whole' / 'found.json').write_text(json.dumps(whole), encoding='utf-8')
            import subprocess
            import sys

            def merging(expect, paths):
                return subprocess.run([sys.executable, str(Path(probe.__file__)), 'merge', '--expect', str(expect), *paths],
                                      capture_output=True, text=True)
            self.assertEqual(merging(1, given[:1]).returncode, 0)
            self.assertEqual(merging(2, given[:1]).returncode, 3)  # a planned slice's file is not there
            partial = merging(2, given[:2])  # there, and not whole
            self.assertEqual(partial.returncode, 3)
            self.assertIn('"counted": 1', partial.stdout.splitlines()[0])
            self.assertIn('commands|cmd/x|y|only vanilla', partial.stdout)
            self.assertFalse(probe.complete({**whole, 'scenarios': True}))
            self.assertFalse(probe.complete({**whole, 'invalid': None}))
            self.assertFalse(probe.complete({**whole, 'groups': [{**whole['groups'][0], 'where': [[]]}]}))
            self.assertTrue(probe.complete(whole))

    def test_a_merge_takes_the_retry_when_the_first_run_stopped_without_a_result(self):
        import tempfile
        from tools import parity_fuzz_probe as probe
        found = json.dumps({'scenarios': 25, 'invalid': [], 'refused': [], 'groups': []})
        with tempfile.TemporaryDirectory() as folder:
            logs = {'fuzz_probe#1.first-run.log': 'parity_fuzz stopped after 1800 s',
                    'fuzz_probe#1.log': '=== found.json ===' + chr(10) + found,
                    'fuzz_probe#2.first-run.log': '=== found.json ===' + chr(10) + found,
                    'fuzz_probe#2.log': 'parity_fuzz stopped after 1800 s',
                    'fuzz_probe#3.first-run.log': 'batch 1', 'fuzz_probe#3.log': 'batch 1',
                    'fuzz_probe#4.first-run.log': 'parity_fuzz stopped after 1800 s',
                    'fuzz_probe#4.log': '6 scenarios: 0 diverge (--smoke)' + chr(10) + 'parity_fuzz exited 0'}
            for name, text in logs.items():
                (Path(folder) / name).write_text(text + chr(10), encoding='utf-8')
            merged = probe.merge(sorted(Path(folder).iterdir()))
        self.assertEqual(merged['totals']['scenarios'], 50)
        self.assertEqual(merged['lost'], ['fuzz_probe#3.first-run.log'])

    def test_a_stack_patched_where_it_lies_is_placed_within_the_bare_items_limit(self):
        from tools import parity_fuzz_data as data
        old = dict(data.PROTOTYPES), dict(data.GAME)
        data.PROTOTYPES.clear()
        data.PROTOTYPES['minecraft:ender_pearl'] = {'minecraft:max_stack_size': 16}
        data.GAME.clear()
        data.GAME['minecraft:max_stack_size'] = ['64']
        data.EDGES = False
        real, data.ITEMS = data.ITEMS, ['ender_pearl']
        try:
            placed = []
            for seed in range(300):
                scenario = data.items(random.Random(seed), seed)
                if any(step.get('how') == 'modify' for step in scenario['steps']):
                    placed += [op['cmd'] for op in fuzz.render(scenario, 0)
                               if op['op'] == 'cmd' and ' with minecraft:ender_pearl ' in op['cmd']]
            self.assertTrue(placed)
            self.assertTrue(all(int(cmd.rsplit(' ', 1)[1]) <= 16 for cmd in placed), placed[:3])
        finally:
            data.EDGES, data.ITEMS = True, real
            for store, kept in zip((data.PROTOTYPES, data.GAME), old):
                store.clear()
                store.update(kept)

    def test_retype_draws_the_same_whatever_order_vanilla_saved_the_fields_in(self):
        from tools import parity_fuzz_scenarios as families
        fields = {'Air': '300s', 'Fire': '-1s', 'PortalCooldown': '0', 'Motion': {'x': '0.0d'}, 'CustomName': '"a"'}
        old = dict(families.KNOWN)
        try:
            made = []
            for order in (list(fields), list(reversed(fields))):
                families.KNOWN.update({'entity': {'pig': {key: fields[key] for key in order}}, 'block': {'barrel': {}}})
                made.append([families.retype(random.Random(seed), seed) for seed in range(40)])
            self.assertEqual(made[0], made[1])
        finally:
            families.KNOWN.clear()
            families.KNOWN.update(old)


class CommandTreeTests(unittest.TestCase):
    tree = {'type': 'root', 'children': {
        'kill': {'type': 'literal', 'executable': True, 'children': {
            'targets': {'type': 'argument', 'parser': 'minecraft:entity', 'executable': True,
                        'properties': {'type': 'entities', 'amount': 'multiple'}}}},
        'effect': {'type': 'literal', 'children': {'give': {'type': 'literal', 'children': {
            'targets': {'type': 'argument', 'parser': 'minecraft:entity',
                        'properties': {'type': 'players', 'amount': 'multiple'}}}}}}}}

    def test_a_walk_never_reaches_command_storage(self):
        from tools import parity_fuzz_commands as commands
        nbt = {'type': 'argument', 'parser': 'minecraft:nbt_compound_tag', 'executable': True}
        tree = {'type': 'root', 'children': {'data': {'type': 'literal', 'children': {'merge': {
            'type': 'literal', 'children': {
                'storage': {'type': 'literal', 'children': {'target': {
                    'type': 'argument', 'parser': 'minecraft:resource_location', 'children': {'nbt': nbt}}}},
                'entity': {'type': 'literal', 'children': {'target': {
                    'type': 'argument', 'parser': 'minecraft:entity',
                    'properties': {'type': 'entities', 'amount': 'single'}, 'children': {'nbt': nbt}}}}}}}}}}
        old = dict(commands.TREE)
        try:
            commands.TREE.clear()
            commands.TREE.update(tree)
            made = [commands.command(random.Random(seed), 7) for seed in range(60)]
            self.assertTrue(any(made))
            self.assertFalse([m for m in made if m and 'storage' in m[0]])
        finally:
            commands.TREE.clear()
            commands.TREE.update(old)

    def test_a_walk_names_its_own_entities_and_never_the_sender_or_a_player(self):
        from tools import parity_fuzz_commands as commands
        old = dict(commands.TREE)
        try:
            commands.TREE.clear()
            commands.TREE.update(self.tree)
            made = [commands.command(random.Random(seed), 7) for seed in range(40)]
            self.assertTrue(all(m is None or m[0].startswith('kill ') and m[1] == 'kill' for m in made))
            self.assertTrue(any(made))  # a bare /kill (the sender) and /effect give <players> are never made
            scenario = commands.commands(random.Random(1), 7)
            texts = [op['cmd'] for op in fuzz.render(scenario, 0) if op['op'] == 'cmd']
            self.assertFalse(any('{u' in text for text in texts))
        finally:
            commands.TREE.clear()
            commands.TREE.update(old)


class ShulkerHitTests(unittest.TestCase):
    def test_live_bullet_scenario_requires_the_vanilla_target_to_take_damage(self):
        scenario = families.shulker(random.Random(10), 10)
        guard, = (op for op in fuzz.render(scenario, 0) if op.get('guard'))
        missed, hit = {'results': [[True, 30]]}, {'results': [[True, 26]]}
        invalid, failure = fuzz.witness('shulker', guard, missed, missed)
        self.assertIsNotNone(invalid)
        self.assertIsNone(failure)

        invalid, failure = fuzz.witness('shulker', guard, hit, missed)
        self.assertIsNone(invalid)
        self.assertEqual(failure[2], [('<hit>', [[True, 26]], [[True, 30]])])

        reduced = fuzz.reduce(scenario, set(fuzz.edits(scenario)))
        retained, = (op for op in fuzz.render(reduced, 0) if op.get('guard'))
        self.assertEqual(retained['cmd'], guard['cmd'])

    def test_shulker_shooting_observes_a_survival_player_then_restores_creative(self):
        scenario = families.shulker(random.Random(0), 0)
        ops = fuzz.render(scenario, 0)
        player_modes = [op for op in ops if op.get('cmd', '').startswith('/gamemode')]
        self.assertEqual([op['cmd'] for op in player_modes], ['/gamemode survival @a', '/gamemode creative @a'])
        ticks = [i for i, op in enumerate(ops) if op['op'] == 'tick']
        self.assertLess(ops.index(player_modes[0]), min(ticks))
        self.assertGreater(ops.index(player_modes[1]), max(ticks))


class ReloadWitnessTests(unittest.TestCase):
    def test_a_reload_gives_the_oracle_time_to_unload_the_chunk(self):
        scenario = {'family': 'persist', 'seed': 1, 'steps': [{'k': 'reload'}, {'k': 'tick', 'n': 4}]}
        ticks = [op for op in fuzz.render(scenario, 0) if op.get('op') == 'tick']
        self.assertEqual([op.get('allow_chunk_io') for op in ticks], [True, None])  # only the reload's wait

    def test_a_reload_witnesses_the_chunk_and_every_entity_summoned_before_it(self):
        scenario = {'family': 'persist', 'seed': 1, 'steps': [
            {'k': 'summon', 'type': 'pig', 'at': [1.5, 0, 1.5], 'n': 0, 'nbt': {}},
            {'k': 'reload'},
            {'k': 'summon', 'type': 'cow', 'at': [2.5, 0, 2.5], 'n': 1, 'nbt': {}},
            {'k': 'reload'}, {'k': 'read_entity', 'n': 1}]}
        pig, cow = (fuzz.uuid_text(fuzz.uuid_words(0, n)) for n in (0, 1))
        chunk = '/execute unless loaded 4096 300 4096'
        guards = [(op['guard'], op['cmd']) for op in fuzz.render(scenario, 0) if op.get('guard')]
        self.assertEqual(guards, [('chunk', chunk), ('entity', f'/execute if entity {pig}'),
                                  ('chunk', chunk), ('entity', f'/execute if entity {pig}'),
                                  ('entity', f'/execute if entity {cow}')])  # the second reload sees both
        entity = {'cmd': f'/execute if entity {pig}', 'guard': 'entity'}
        gone = {'cmd': entity['cmd'], 'results': [[False, 0]]}
        kept = {'cmd': entity['cmd'], 'results': [[True, 1]]}
        self.assertEqual(fuzz.witness('persist', entity, gone, gone), (None, None))
        self.assertEqual(fuzz.witness('persist', entity, kept, kept)[0],
                         'vanilla kept the entity loaded: [[True, 1]]')
        self.assertEqual(fuzz.witness('persist', entity, gone, kept)[1][0], 'persist|entity|<unload>')
        chunk_op = {'cmd': chunk, 'guard': 'chunk'}
        chunk_stayed = {'cmd': chunk, 'results': [[False, 0]]}  # `/execute unless loaded` failed
        self.assertEqual(fuzz.witness('persist', chunk_op, chunk_stayed, chunk_stayed)[0],
                         'vanilla kept the chunk loaded: [[False, 0]]')

    def test_each_guard_names_its_signature_so_divergences_do_not_merge(self):
        # An entity guard and a chunk guard diverge the same way, but must not be grouped as one bug.
        entity = {'cmd': '/execute if entity 00000000-0000-0000-0000-000000000001', 'guard': 'entity'}
        chunk = {'cmd': '/execute unless loaded 4096 300 4096', 'guard': 'chunk'}
        gone, kept = {'results': [[False, 0]]}, {'results': [[True, 1]]}
        entity_invalid, entity_failure = fuzz.witness('spawn', entity, gone, kept)
        chunk_invalid, chunk_failure = fuzz.witness('spawn', chunk, gone, kept)
        self.assertIsNone(entity_invalid)  # [[False, 0]] is the valid reload for an entity guard
        self.assertEqual(entity_failure[0], 'spawn|entity|<unload>')
        self.assertEqual(chunk_failure[0], 'spawn|chunk|<unload>')  # chunk behavior is unchanged
        self.assertNotEqual(entity_failure[0], chunk_failure[0])
        self.assertEqual(fuzz.signatures({'invalid': None, 'failures': [entity_failure]}),
                         ['spawn|entity|<unload>'])
        self.assertEqual(fuzz.signatures({'invalid': None, 'failures': [chunk_failure]}),
                         ['spawn|chunk|<unload>'])


if __name__ == '__main__':
    unittest.main()
