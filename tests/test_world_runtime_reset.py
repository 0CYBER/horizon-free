"""The actual world-entry reset must invalidate previously loaded player progress."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def run_reset(source):
    start = source.index("        self.persist = Default::default();", source.index("    fn open_world("))
    reset = source[start:source.index("        // RandomSequences", start)]
    program = """
#[derive(Default)] struct Game {persist:bool,score:bool,fns:bool,bosses:bool,adv:bool,stats:bool}
impl Game {fn enter_world(&mut self) {RESET}}
fn main() {
 let mut game=Game {persist:true,score:true,fns:true,bosses:true,adv:true,stats:true};
 game.enter_world();
 assert!(!game.adv,"loaded advancement progress must not survive world entry");
 assert!(!game.stats,"loaded statistics must not survive world entry");
}
""".replace("RESET", reset)
    with tempfile.TemporaryDirectory(prefix="horizon-world-reset-") as directory:
        path = Path(directory)
        (path / "probe.rs").write_text(program)
        subprocess.run(["rustc", "--edition=2024", str(path / "probe.rs"), "-o", str(path / "probe")], check=True, capture_output=True)
        return subprocess.run([str(path / "probe")], capture_output=True, text=True)


class WorldRuntimeResetTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "rustc is required")
    def test_world_entry_invalidates_loaded_progress(self):
        result = run_reset((ROOT / "crates/horizon-client/src/game.rs").read_text())
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
