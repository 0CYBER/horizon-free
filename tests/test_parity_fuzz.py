import json
import random
import unittest
from pathlib import Path

from tools import parity_fuzz as fuzz
from tools import parity_fuzz_scenarios as families


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
