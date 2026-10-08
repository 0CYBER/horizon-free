import unittest
import tempfile
from pathlib import Path
from tools import test_block_survival_parity as survival


class SurvivalCheckpointTests(unittest.TestCase):
    def test_resume_rejects_changed_sources_environment_inputs_and_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fixture=root/'fixture';binary=root/'binary';path=root/'checkpoint.json'
            fixture.write_bytes(b'original input');binary.write_bytes(b'original executable')
            sources={'source.rs':'original'};environment={'java':'pinned'}
            checkpoint={'schema':1,'sources_sha256':sources,'environment':environment,'fixtures_sha256':{str(fixture):survival.lab.digest(fixture,'sha256')},'artifacts_sha256':{str(binary):survival.lab.digest(binary,'sha256')}}
            survival.save_checkpoint(path,checkpoint)
            self.assertEqual(survival.load_checkpoint(path,sources,environment),checkpoint)
            for changed_sources,changed_environment in [({'source.rs':'edited'},environment),(sources,{'java':'different'})]:
                with self.assertRaises(ValueError):survival.load_checkpoint(path,changed_sources,changed_environment)
            for artifact in [fixture,binary]:
                original=artifact.read_bytes();artifact.write_bytes(b'changed')
                with self.assertRaises(ValueError):survival.load_checkpoint(path,sources,environment)
                artifact.unlink()
                with self.assertRaises(ValueError):survival.load_checkpoint(path,sources,environment)
                artifact.write_bytes(original)
            self.assertEqual(survival.load_checkpoint(path,sources,environment),checkpoint)


class BlockSurvivalCorpusTests(unittest.TestCase):
    def test_definition_dispatch_preserves_virtual_support_methods(self):
        classes = {'a': 'VegetationBlock', 'b': 'VegetationBlock', 'c': 'VegetationBlock', 'd': 'DoublePlantBlock', 'e': 'DoublePlantBlock'}
        placement = {'a': 'VegetationBlock', 'b': 'StemBlock', 'c': 'UnknownVegetationBlock', 'd': 'VegetationBlock', 'e': 'UnknownPlantBlock'}
        tags = {'b': {'stemSupportBlocks': 'minecraft:supports_melon_stem', 'fruitSupportBlocks': 'minecraft:wrong_for_survival'}}
        rules = survival.bind_definitions(classes, placement, tags)
        self.assertEqual(rules, {'a': 'TagBelow:minecraft:supports_vegetation', 'b': 'TagBelow:minecraft:supports_melon_stem', 'd': 'DoublePlant'})
        with self.assertRaises(KeyError):
            survival.bind_definitions(classes, placement, {})

    def test_branching_neighbor_sequences_and_candidate_properties(self):
        names=['air','stone','end_stone','chorus_plant','chorus_flower','scaffolding','bell','pitcher_crop']
        states=[{'id':'minecraft:'+name,'properties':{}} for name in names]
        states += [dict(states[-2],properties={'attachment':'ceiling','facing':'north'}),dict(states[-1],properties={'half':'upper'})]
        facts={'states':states,'defaults':{'minecraft:'+name:index for index,name in enumerate(names)}}
        definitions={states[i]['id']:rule for i,rule in [(3,'ChorusPlant'),(4,'ChorusFlower'),(5,'Scaffolding'),(6,'Bell'),(7,'PitcherCrop')]}
        seen=set();long_failures=set();brightness=set();upper_brightness=set()
        for fixture in survival.corpus(facts,definitions):
            seen.add(fixture['state'])
            if len(fixture['neighbors'])==10:long_failures.add(fixture['fail'])
            if 'brightness' in fixture:
                brightness.add(fixture['brightness'])
                if fixture['state']==9:upper_brightness.add(fixture['brightness'])
        self.assertEqual(seen,{3,4,5,6,7,8,9})
        self.assertEqual(long_failures,{-1,*range(10)})
        self.assertTrue({7,8,9}.issubset(brightness))
        self.assertEqual(upper_brightness,brightness)

    def test_supported_candidates_and_query_transitions(self):
        names = ['minecraft:air', 'minecraft:stone', 'minecraft:shulker_box', 'minecraft:moving_piston', 'minecraft:pale_moss_carpet', 'minecraft:unsupported']
        facts = {'states': [{'id': name, 'properties': {'bottom': 'true'}} for name in names], 'defaults': dict(zip(names, range(len(names))))}
        definitions = {names[0]: 'Always', names[1]: 'RigidOrCenterBelow', names[4]: 'MossCarpet'}
        fixtures = list(survival.corpus(facts, definitions))
        self.assertEqual(fixtures, list(survival.corpus(facts, definitions)))
        self.assertEqual({f['state'] for f in fixtures}, {0, 1, 4})
        self.assertEqual(len([f for f in fixtures if f['state'] == 0]), 1)
        self.assertTrue(any(f['neighbors'] == [2, 3] for f in fixtures))
        self.assertTrue(any(f['fail'] == 1 and f['entity_fail'] == 0 for f in fixtures))
        self.assertEqual({f['entity'] for f in fixtures}, {'none', 'closed', 'opened'})
        self.assertTrue(any(-2147483648 in f['position'] for f in fixtures))
        for candidate in [1, 4]:
            self.assertTrue(all(any(f['state'] == candidate and f['neighbors'] == [neighbor] and f['fail'] == -1 for f in fixtures) for neighbor in range(len(names))))


if __name__ == '__main__':
    unittest.main()
