"""Regression tests for tools.graphics_bench.compare (finding p04)."""
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import graphics_bench as gb


def _scene(avg_fps=60.0):
    return {
        "avg_fps": avg_fps,
        "low_1pct_fps": 50.0,
        "p99_ms": 16.0,
        "cpu": {"avg_ms": 5.0},
        "gpu": {"avg_ms": 7.0},
    }


class CompareMissingSceneTests(unittest.TestCase):
    def test_scene_absent_from_after_is_reported_not_dropped(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / "benchmarks" / "graphics"
            reports.mkdir(parents=True)
            before = {"distances": {"8": {"renderer": "x", "alpha": _scene(), "beta": _scene()}}}
            after = {"distances": {"8": {"renderer": "x", "alpha": _scene(90.0)}}}
            (reports / "before.json").write_text(json.dumps(before))
            (reports / "after.json").write_text(json.dumps(after))

            out = io.StringIO()
            with mock.patch.object(gb, "ROOT", root), contextlib.redirect_stdout(out):
                gb.compare("before", "after")
            table = out.getvalue()

            # The baseline-only scene must be called out instead of vanishing.
            self.assertIn("beta", table)
            self.assertIn("missing", table)
            # A scene present in both runs still gets a normal comparison row.
            self.assertIn("alpha", table)
            self.assertIn("60 → 90", table)
            # The renderer string is not a scene and must not be tabulated.
            self.assertNotIn("renderer", table)


if __name__ == "__main__":
    unittest.main()
