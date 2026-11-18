import contextlib
import gzip
import io
import struct
import sys
import tempfile
import unittest
from pathlib import Path, PureWindowsPath
from unittest.mock import patch
ROOT = Path(__file__).resolve().parent.parent

# The CLI adds tools/ to sys.path; do not let it shadow unittest modules.
with patch.object(sys, 'path', sys.path.copy()):
    from tools import test_save_roundtrip_parity as roundtrip

def tag(kind, name, payload):
    raw = name.encode()
    return bytes([kind]) + struct.pack('>H', len(raw)) + raw + payload


def compound(*tags):
    return b''.join(tags) + b'\0'


def integer(name, value):
    return tag(3, name, struct.pack('>i', value))


class WindowsPath(type(Path())):
    """Keep real file access while relative paths have Windows separators."""
    def relative_to(self, *args, **kwargs):
        return PureWindowsPath(*super().relative_to(*args, **kwargs).parts)

    def __truediv__(self, key):
        return super().__truediv__(PureWindowsPath(key).as_posix())


class SaveRoundtripPathsTests(unittest.TestCase):
    def test_windows_paths_accept_expected_changes_but_retain_real_differences(self):
        with tempfile.TemporaryDirectory() as folder:
            vanilla, horizon = WindowsPath(folder) / 'vanilla', WindowsPath(folder) / 'horizon'
            for world, rule, health, slot, score in (
                (vanilla, 24, 20.0, 0, 1),
                (horizon, 30, 18.0, 1, 2),
            ):
                files = {
                    'level.dat': compound(),
                    'data/minecraft/game_rules.dat': compound(
                        tag(10, 'data', compound(integer('minecraft:max_entity_cramming', rule))),
                    ),
                    'players/data/player.dat': compound(
                        tag(5, 'Health', struct.pack('>f', health)),
                        integer('SelectedItemSlot', slot),
                    ),
                    'data/minecraft/custom.dat': compound(integer('score', score)),
                }
                for rel, payload in files.items():
                    path = world / rel
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(gzip.compress(tag(10, '', payload)))
            # This fixture has no region entities or leash holders.
            with patch.object(roundtrip.scene, 'LEADS', 0):
                report = roundtrip.compare_worlds(vanilla, horizon)
            self.assertCountEqual(report, [
                ('data/minecraft/custom.dat.score', 1, 2),
                ('players/data/player.dat.Health', ('f', 20.0), ('f', 18.0)),
                ('players/data/player.dat.SelectedItemSlot', 0, 1),
            ])
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(roundtrip.summarize(report), 2)


class SaveRoundtripTimersTests(unittest.TestCase):
    def test_elapsed_despawn_timers_do_not_hide_other_saved_state_changes(self):
        report = [
            ('entity.DespawnDelay @minecraft:trader_llama#57', 47384, 47375),
            ('entity.Passengers[].DespawnDelay @minecraft:trader_llama#58', 100, 91),
            ('entity.PersistenceRequired @minecraft:trader_llama#57', 0, 1),
        ]
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(roundtrip.summarize(report), 1)


class SaveRoundtripDatFileSetTests(unittest.TestCase):
    """compare_worlds() must enumerate .dat files from both worlds, not just vanilla's."""

    def make_worlds(self, vanilla_files, horizon_files):
        tmp = tempfile.TemporaryDirectory(prefix='save-roundtrip-')
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        worlds = []
        for name, files in (('v', vanilla_files), ('h', horizon_files)):
            world = root / name
            for rel, payload in files.items():
                path = world / rel
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(gzip.compress(tag(10, '', payload)))
            worlds.append(world)
        return worlds

    def compare(self, vanilla, horizon):
        # These fixtures have no region entities or leash holders.
        with patch.object(roundtrip.scene, 'LEADS', 0):
            return roundtrip.compare_worlds(vanilla, horizon)

    def rows(self, report, rel):
        return [row for row in report if row[0] == rel or str(row[0]).startswith(rel + '.')]

    def test_horizon_only_dat_file_is_reported_as_missing_from_vanilla(self):
        shared = {'level.dat': compound(), 'data/shared.dat': compound(integer('x', 1))}
        horizon = dict(shared)
        horizon['data/horizon_only.dat'] = compound(integer('x', 1))
        vanilla, h = self.make_worlds(shared, horizon)

        report = self.compare(vanilla, h)

        self.assertEqual(self.rows(report, 'data/horizon_only.dat'),
                         [('data/horizon_only.dat', 'missing from vanilla', 'present')])
        self.assertEqual(self.rows(report, 'data/shared.dat'), [])

    def test_vanilla_only_dat_file_is_still_reported_as_missing_from_horizon(self):
        vanilla, h = self.make_worlds(
            {'level.dat': compound(), 'data/vanilla_only.dat': compound(integer('x', 1))},
            {'level.dat': compound()})

        report = self.compare(vanilla, h)

        self.assertEqual(self.rows(report, 'data/vanilla_only.dat'),
                         [('data/vanilla_only.dat', 'present', 'missing from Horizon')])

    def test_matched_dat_files_still_have_their_nbt_compared(self):
        vanilla, h = self.make_worlds(
            {'level.dat': compound(), 'players/data/player.dat': compound(integer('score', 3))},
            {'level.dat': compound(), 'players/data/player.dat': compound(integer('score', 4))})

        report = self.compare(vanilla, h)

        self.assertEqual(self.rows(report, 'players/data/player.dat.score'),
                         [('players/data/player.dat.score', 3, 4)])

    def test_horizon_only_level_dat_is_reported(self):
        vanilla, h = self.make_worlds({}, {'level.dat': compound(integer('x', 1))})

        report = self.compare(vanilla, h)

        self.assertEqual(self.rows(report, 'level.dat'),
                         [('level.dat', 'missing from vanilla', 'present')])

    def test_identical_worlds_report_no_dat_file_differences(self):
        files = {'level.dat': compound(), 'data/other.dat': compound(integer('x', 1))}
        vanilla, h = self.make_worlds(files, files)

        report = self.compare(vanilla, h)

        self.assertEqual(self.rows(report, 'level.dat'), [])
        self.assertEqual(self.rows(report, 'data/other.dat'), [])


if __name__ == '__main__':
    unittest.main()
