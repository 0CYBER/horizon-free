from pathlib import Path
import itertools
import json
import tempfile
import unittest
from unittest import mock

from tools import test_worldgen_parity as parity


class WorldgenRunnerTests(unittest.TestCase):
    def test_failed_verification_removes_previous_success(self):
        with tempfile.TemporaryDirectory() as temp:
            report = Path(temp)/"rust-worldgen-parity/report.json"
            report.parent.mkdir()
            report.write_text('{"previous_success":true}')
            with mock.patch.object(parity.lab,"LOCAL",Path(temp)), mock.patch("sys.argv",["parity"]), \
                 mock.patch.object(parity.base,"verify_artifacts",side_effect=ValueError("corrupt artifact")):
                with self.assertRaisesRegex(ValueError,"corrupt artifact"):
                    parity.main()
            self.assertFalse(report.exists())

    def test_corpus_keeps_numeric_text_and_checks_all_sampling_modes(self):
        resources = {"fixture":"{\"base_octave\":-7}"}
        commands = list(parity.corpus(resources))
        self.assertEqual(commands,list(parity.corpus(resources)))
        self.assertEqual(len(commands),599)
        self.assertEqual({tuple(line.split(" ",4)[2:4]) for line in commands}, {(str(a),str(b)) for a in [0,1] for b in [0,1]})
        text = "\n".join(commands)
        for retained in ["4294967295","-4294967297.9","1.9999999999999999999","[-0]","[-0.0]","0.000009999999747378751","0.000009999999747378752"]:
            self.assertIn(retained,text)

    def test_different_client_registry_is_rejected_before_building(self):
        with tempfile.TemporaryDirectory() as temp:
            with mock.patch.object(parity.lab,"LOCAL",Path(temp)), mock.patch("sys.argv",["parity"]), \
                 mock.patch.object(parity.base,"verify_artifacts",return_value=({}, {"server":"server", "client":"client"}, [])), \
                 mock.patch.object(parity,"noise_resources",side_effect=[{"a":"{}"},{"a":"{\"different\":true}"}]), \
                 mock.patch.object(parity.base,"run") as run:
                with self.assertRaisesRegex(ValueError,"resources differ"):
                    parity.main()
                run.assert_not_called()

    def test_density_and_settings_mismatches_are_rejected_before_building(self):
        for kind in ["density", "settings"]:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temp:
                matching = [{"a":"{}"},{"a":"{}"}]
                mismatch = [{"a":"{}"},{"a":"{\"changed\":true}"}]
                outputs = mismatch if kind == "density" else matching + mismatch
                with mock.patch.object(parity.lab,"LOCAL",Path(temp)), mock.patch("sys.argv",["parity"]), \
                     mock.patch.object(parity.base,"verify_artifacts",return_value=({}, {"server":"server", "client":"client"}, [])), \
                     mock.patch.object(parity,"noise_resources",return_value={"a":"{}"}), \
                     mock.patch.object(parity,"registry_resources",side_effect=outputs), \
                     mock.patch.object(parity.base,"run") as run:
                    with self.assertRaisesRegex(ValueError,"resources differ"):
                        parity.main()
                    run.assert_not_called()

    def test_climate_corpus_is_deterministic_and_retains_nonfinite_inputs(self):
        commands = list(parity.climate_corpus())
        self.assertEqual(commands,list(parity.climate_corpus()))
        self.assertEqual(len(commands),4116)
        text = "\n".join(commands)
        for retained in ["7f800001","ff800000","80000000",str(-2**63),str(2**63-1)]:
            self.assertIn(retained,text)

    def test_index_corpus_preserves_ordered_warm_tie_queries(self):
        commands = list(itertools.islice(parity.index_corpus(),5))
        self.assertEqual(commands,list(itertools.islice(parity.index_corpus(),5)))
        values = json.loads(commands[0].split(" ",2)[2])
        self.assertEqual([v["coordinates"][0] for v in values["entries"]],[[-10,-10],[10,10]])
        self.assertEqual([v[0] for v in values["targets"]],[10,0,-10,0,10,0])
        self.assertEqual(json.loads(commands[-1].split(" ",2)[2])["entries"],[])

    def test_climate_codec_corpus_retains_precision_and_missing_fields(self):
        commands = list(parity.climate_codec_corpus())
        self.assertEqual(commands,list(parity.climate_codec_corpus()))
        self.assertEqual(len(commands),208)
        self.assertIn("climate_parameter 2.0000001192092895507812500000000001",commands)
        self.assertIn("climate_parameter [0,-0]",commands)
        point_inputs = [json.loads(command.split(" ",1)[1]) for command in commands if command.startswith("climate_point ")]
        self.assertTrue(any("offset" not in value for value in point_inputs))

    def test_settings_corpus_keeps_json_and_runtime_construction_separate(self):
        resources = {"fixture":'{"noise":{"min_y":-64,"height":384}}'}
        commands = list(parity.settings_corpus(resources))
        self.assertEqual(commands,list(parity.settings_corpus(resources)))
        self.assertEqual(len(commands),393)
        self.assertIn("settings_create -2147483648 -2147483648",commands)
        self.assertIn('noise_settings {"min_y":16.9999999999999999999,"height":16}',commands)

    def test_aquifer_corpus_keeps_nan_density_and_fluid_cutoff_boundaries(self):
        commands = list(parity.aquifer_corpus())
        self.assertEqual(commands,list(parity.aquifer_corpus()))
        self.assertEqual(len(commands),9136)
        self.assertIn("aquifer 63 0 0 -54 0 7ff0000000000001",commands)
        self.assertIn("aquifer -64 4 0 -65 0 8000000000000000",commands)

    def test_noise_aquifer_corpus_preserves_stateful_queries_and_all_failures(self):
        resources={"minecraft:overworld":"{}"}
        commands=list(parity.noise_aquifer_corpus("bundle.json",resources))
        self.assertEqual(commands,list(parity.noise_aquifer_corpus("bundle.json",resources)))
        fixtures=[json.loads(command.split(" ",5)[5]) for command in commands]
        self.assertEqual({tuple(f["fail"]) for f in fixtures if "fail" in f},
                         {(field,occurrence) for field in range(6) for occurrence in [1,2,8]})
        for field in range(6):
            overrides={f["overrides"][field] for f in fixtures if "overrides" in f}
            self.assertTrue({"7fc00000","7f800001","7f800000","ff800000","80000000"} <= overrides)
        text="\n".join(commands)
        self.assertIn("7ff0000000000001",text)
        self.assertIn("minecraft:missing",text)
        self.assertTrue(any(f["fluid"]==7 for f in fixtures))
        self.assertTrue(any(f["steps"]==[4,2,4] for f in fixtures))

    def test_tag_corpus_covers_cycles_replacement_and_codec_numeric_types(self):
        commands=list(parity.tag_corpus({"minecraft:example":'{"values":["stone"]}'}))
        self.assertEqual(commands,list(parity.tag_corpus({"minecraft:example":'{"values":["stone"]}'})))
        self.assertIn('tag_entry {"id":"stone","required":0.5}',commands)
        self.assertIn('tag_entry "#"',commands)
        fixtures=[json.loads(command[5:]) for command in commands if command.startswith("tags ")]
        self.assertTrue(any('"replace":true' in row[2] for f in fixtures for row in f["files"]))
        self.assertTrue(any('"#a"' in row[2] and row[0]=="b" for f in fixtures for row in f["files"]))
        self.assertTrue(any(row[2]=="{}" for f in fixtures for row in f["files"]))

    def test_different_builtin_biome_tables_fail_before_rust_build(self):
        with tempfile.TemporaryDirectory() as temp:
            exports = []
            def run(command, **kwargs):
                if "dev.horizonlab.parity.VanillaBiomeExport" in command:
                    destination = Path(command[-1])
                    destination.write_text(json.dumps({"minecraft:fixture":str(len(exports))}))
                    exports.append(destination)
                self.assertNotIn("cargo",command)
            resources = [
                {"data/minecraft/worldgen/density_function/a.json":"0"},
                {"data/minecraft/worldgen/noise_settings/a.json":"{}"},
            ]
            with mock.patch.object(parity.lab,"LOCAL",Path(temp)), mock.patch("sys.argv",["parity"]), \
                 mock.patch.object(parity.base,"verify_artifacts",return_value=({"minecraft":{"java_major":25}}, {"server":"server", "client":"client"}, [])), \
                 mock.patch.object(parity,"noise_resources",return_value={"data/minecraft/worldgen/noise/a.json":"{}"}), \
                 mock.patch.object(parity,"registry_resources",side_effect=[resources[0],resources[0],resources[1],resources[1]]), \
                 mock.patch.object(parity,"tag_resources",return_value={"minecraft:a":'{"values":[]}'}), \
                 mock.patch.object(parity.base,"source_hashes",return_value={}), \
                 mock.patch.object(parity.base,"run",side_effect=run):
                with self.assertRaisesRegex(ValueError,"biome parameter tables differ"):
                    parity.main()
            self.assertEqual(len(exports),2)


if __name__ == "__main__":
    unittest.main()
