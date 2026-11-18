import random
import unittest

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
