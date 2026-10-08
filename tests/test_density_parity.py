from collections import Counter
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import test_density_parity as parity


class DensityRunnerTests(unittest.TestCase):
    def test_failed_reference_verification_removes_previous_success(self):
        with tempfile.TemporaryDirectory() as temp:
            report = Path(temp) / "rust-density-parity/report.json"
            report.parent.mkdir()
            report.write_text('{"previous_success":true}')
            with mock.patch.object(parity.lab, "LOCAL", Path(temp)), mock.patch("sys.argv", ["parity"]), \
                 mock.patch.object(parity.test_rust_parity, "verify_artifacts", side_effect=ValueError("corrupt artifact")):
                with self.assertRaisesRegex(ValueError, "corrupt artifact"):
                    parity.main()
            self.assertFalse(report.exists())

    def test_corpus_replays_valid_graph_references_and_edge_cases(self):
        commands = list(parity.corpus())
        self.assertEqual(commands, list(parity.corpus()))
        counts = Counter(line.split()[0] for line in commands)
        self.assertEqual(counts["gradient"], 2041)
        self.assertEqual(counts["shift"], 1200)
        self.assertGreater(counts["query"], 140000)
        self.assertEqual(counts["bounds"], 41176)
        self.assertEqual(counts["offset"], 816)
        self.assertEqual(counts["blended"], 192)
        self.assertEqual(counts["blendfail"], 10)
        self.assertEqual(counts["blendfbm"], 256)
        self.assertEqual(counts["counted"], 3120)
        self.assertEqual(counts["context"], 34805)
        self.assertEqual(counts["lookup"], 10000)
        self.assertEqual(counts["slice"], 384)
        self.assertEqual(counts["interpolate"], 384)
        self.assertEqual(counts["surface"], 384)
        self.assertEqual(counts["sppoints"], 768)
        self.assertEqual(counts["spconstant"], 78)
        self.assertEqual(counts["spsampler"] + counts["spfail"], 334)
        self.assertGreaterEqual(counts["spfail"], 2)
        self.assertEqual(counts["sprange"], 13536)
        self.assertEqual(counts["greset"], 368)
        self.assertEqual(counts["gcompile"], 18282)
        self.assertEqual(counts["power"], 1456)
        self.assertEqual(counts["gpow"], 1552)
        self.assertEqual(counts["round"], 1576)
        self.assertEqual(counts["ground"], 880)
        self.assertEqual(counts["gsconstant"], 1120)
        self.assertEqual(counts["gspoints"], 1456)
        self.assertEqual(counts["gspline"], 2160)
        self.assertEqual(counts["gseed"], 70)
        self.assertEqual(counts["gready"], 70)
        self.assertEqual(counts["gregister"], 326)
        self.assertEqual(counts["gnoise"], 838)
        self.assertEqual(counts["goffset"], 262)
        self.assertEqual(counts["gblended"], 64)
        self.assertEqual(counts["gend"], 128)
        self.assertEqual(counts["gref"], 288)
        self.assertEqual(counts["gnamed"], 224)
        self.assertEqual(counts["glookup"], 64)
        self.assertEqual(counts["wfield"], 96)
        self.assertEqual(counts["blenddensity"], 96)
        self.assertEqual(counts["gwfield"], 96)
        self.assertEqual(counts["gblenddensity"], 176)
        self.assertEqual(counts["wcontext"], 20416)
        self.assertEqual(counts["ginstance"], 640)
        self.assertEqual(counts["ginterval"], 1024)
        self.assertEqual(counts["gdistance"], 128)
        self.assertEqual({line.split()[1] for line in commands if line.startswith("bounds ")}, set("012345"))
        samplers = noises = splines = graph_nodes = graph_splines = graph_noises = 0
        tilings = set()
        noise_modes = set()
        shift_modes = set()
        graph_ready = False
        named_functions = set()
        for command in commands:
            fields = command.split()
            if fields[0] in ["greset", "gseed"]:
                graph_nodes = 0
                graph_splines = 0
                graph_noises = 0
                graph_ready = False
                named_functions.clear()
            elif fields[0] in ["gref", "gnamed", "glookup"]:
                if fields[0] == "gnamed":
                    self.assertNotIn(fields[1], named_functions)
                    self.assertLess(int(fields[2]), graph_nodes)
                    named_functions.add(fields[1])
                elif fields[0] == "glookup":
                    self.assertIn(fields[1], named_functions)
                else:
                    self.assertLess(int(fields[1]), graph_nodes)
                graph_nodes += 1
            elif fields[0] == "gregister":
                self.assertFalse(graph_ready)
                self.assertIn(":", fields[1])
                self.assertEqual(len(fields), 7 + int(fields[6]))
                graph_noises += 1
            elif fields[0] == "gready":
                self.assertFalse(graph_ready)
                self.assertGreater(graph_noises, 0)
                graph_ready = True
            elif fields[0] == "ginstance":
                self.assertTrue(graph_ready)
                self.assertLess(int(fields[1]), graph_noises)
            elif fields[0] == "gnoise":
                self.assertTrue(graph_ready)
                self.assertLess(int(fields[1]), graph_noises)
                for reference in map(int,fields[4:7]):
                    self.assertLess(reference, graph_nodes)
                graph_nodes += 1
            elif fields[0] == "goffset":
                self.assertLess(int(fields[1]), graph_noises)
                self.assertIn(fields[2], ["0", "1", "2"])
                graph_nodes += 1
            elif fields[0] == "gsconstant":
                graph_splines += 1
            elif fields[0] == "gspoints":
                self.assertLess(int(fields[1]), graph_nodes)
                count = int(fields[2])
                self.assertEqual(len(fields), 3 + 3*count)
                for reference in map(int, fields[5::3]):
                    self.assertLess(reference, graph_splines)
                graph_splines += 1
            elif fields[0] == "gspline":
                self.assertLess(int(fields[1]), graph_splines)
                graph_nodes += 1
            elif fields[0] == "gbind":
                self.assertLess(int(fields[1]), samplers)
                graph_nodes += 1
            elif fields[0] in ["gconst", "ggradient", "gdistance", "gblended", "gend", "gwfield"]:
                graph_nodes += 1
            elif fields[0] == "ginterval":
                count = int(fields[2])
                self.assertEqual(len(fields), 4 + 2*count)
                for reference in map(int, fields[1:2] + fields[3+count:]):
                    self.assertLess(reference,graph_nodes)
                graph_nodes += 1
            elif fields[0] in ["gunary", "gbinary", "ground", "gpow", "gclamp", "gchoice", "glerp", "gslice", "gcache", "ginterp", "gsurface", "gblenddensity"]:
                references = {
                    "gunary": fields[2:3], "gbinary": fields[2:4], "ground": fields[2:4], "gpow": fields[1:3], "gclamp": fields[1:2],
                    "gchoice": fields[1:2] + fields[4:6], "glerp": fields[1:4], "gslice": fields[3:4],
                    "gcache": fields[1:2], "gblenddensity": fields[1:2], "ginterp": fields[1:2], "gsurface": fields[1:3],
                }[fields[0]]
                for reference in map(int,references):
                    self.assertLess(reference,graph_nodes)
                graph_nodes += 1
            elif fields[0] in ["gcompile", "gtry"]:
                self.assertLess(int(fields[2]),graph_nodes)
            elif fields[0] in ["ginfo", "gsame"]:
                for reference in map(int,fields[1:]):
                    self.assertLess(reference,graph_nodes)
            elif fields[0] in ["cache", "counted"]:
                self.assertLess(int(fields[-1]), samplers)
            elif fields[0] == "spconstant":
                splines += 1
            elif fields[0] == "sppoints":
                self.assertLess(int(fields[1]), samplers)
                self.assertEqual(len(fields), 3 + int(fields[2])*3)
                for reference in map(int, fields[5::3]):
                    self.assertLess(reference, splines)
                splines += 1
            elif fields[0] in ["spsampler", "spfail", "sprange"]:
                self.assertLess(int(fields[1]), splines)
            elif fields[0] == "surface":
                for reference in map(int, fields[2:4]):
                    self.assertLess(reference, samplers)
                self.assertGreater(int(fields[5]), 0)
            elif fields[0] == "interpolate":
                self.assertLess(int(fields[1]), samplers)
                self.assertGreater(int(fields[2]), 0)
                self.assertGreater(int(fields[3]), 0)
            elif fields[0] == "slice":
                self.assertIn(fields[1], list("0123"))
                self.assertLess(int(fields[2]), samplers)
            elif fields[0] == "offset":
                self.assertIn(fields[1], ["0", "1", "2"])
                self.assertLess(int(fields[2]), noises)
            elif fields[0] == "interval":
                count = int(fields[3])
                references = [fields[2]] + fields[4+count:]
                self.assertEqual(len(references), count + 2)
                if fields[1] == "1":
                    self.assertEqual(count, 1)
                for reference in map(int, references):
                    self.assertGreaterEqual(reference, 0)
                    self.assertLess(reference, samplers)
            elif fields[0] in ["choice", "lerp"]:
                mode = int(fields[1])
                references = [fields[2]]
                if fields[0] == "choice" and mode == 0:
                    references += fields[5:7]
                elif fields[0] == "lerp":
                    if mode != 1:
                        references.append(fields[3])
                    if mode != 2:
                        references.append(fields[4])
                for reference in map(int, references):
                    self.assertGreaterEqual(reference, 0)
                    self.assertLess(reference, samplers)
            elif fields[0] == "wcontext":
                self.assertIn(fields[1], ["0", "1"])
                self.assertIn(fields[5], ["0", "1"])
                for reference in map(int,fields[2:5]):
                    self.assertGreaterEqual(reference, -1)
                    self.assertLess(reference, samplers)
            elif fields[0] == "wfield":
                self.assertIn(fields[1], ["0", "1", "2"])
                self.assertGreaterEqual(int(fields[2]), -1)
                self.assertLess(int(fields[2]), samplers)
            elif fields[0] == "blenddensity":
                self.assertLess(int(fields[1]), samplers)
            elif fields[0] == "round":
                self.assertIn(fields[1], ["0", "1", "2", "3"])
                self.assertGreaterEqual(int(fields[2]), 0)
                self.assertLess(int(fields[2]), samplers)
                self.assertGreaterEqual(int(fields[3]), -1)
                self.assertLess(int(fields[3]), samplers)
            elif fields[0] == "power":
                self.assertIn(fields[1], ["0", "1", "2"])
                references = fields[2:4] if fields[1] == "0" else fields[3:4] if fields[1] == "1" else fields[2:3]
                for reference in map(int,references):
                    self.assertGreaterEqual(reference, 0)
                    self.assertLess(reference, samplers)
            elif fields[0] in ["unary", "binary"]:
                references = fields[2:3] if fields[0] == "unary" else fields[2:4]
                for reference in map(int, references):
                    self.assertGreaterEqual(reference, 0)
                    self.assertLess(reference, samplers)
            elif fields[0] == "shift":
                self.assertLess(int(fields[1]), noises)
                for reference in map(int, fields[2:]):
                    self.assertGreaterEqual(reference, -1)
                    self.assertLess(reference, samplers)
                shift_modes.add(int(fields[3]) >= 0)
            elif fields[0] == "noise":
                noise_modes.add(tuple(fields[1:3]))
                noises += 1
            elif fields[0] == "gradient":
                tilings.add(tuple(fields[1:3]))
            elif fields[0] in ["volume", "query"]:
                self.assertLess(int(fields[1]), samplers)
            if fields[0] in ["constant", "gradient", "noise", "shift", "unary", "binary", "round", "power", "choice", "lerp", "interval", "offset", "blended", "distance", "cache", "counted", "slice", "interpolate", "surface", "spsampler", "gcompile", "wfield", "blenddensity"]:
                samplers += 1
        self.assertEqual(shift_modes, {False, True})
        self.assertEqual(tilings, {(str(axis), str(mode)) for axis in range(3) for mode in range(3)})
        self.assertEqual(noise_modes, {(kind, stream) for kind in ["perlin", "simplex", "smeared", "normal"] for stream in ["legacy", "xoroshiro"]})


if __name__ == "__main__":
    unittest.main()
