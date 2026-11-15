#!/usr/bin/env python3
"""Kill three publication mutants with real resource-gated Game regressions.

Run only in a private checkout with no concurrent Cargo build. Resources must be
an already prepared private copy. Each edited source is restored in finally.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

import lab

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--resources", type=Path, required=True)
args = parser.parse_args()
resources = args.resources.resolve()
shared = os.environ.get("HORIZON_SHARED_SOURCE") or Path.home() / "code/horizon-parity/shared-local"
if resources.is_relative_to(Path(shared)):
    parser.error("use a private resource copy, not the read-only shared tree")
logs = ROOT / ".local/row4-mutants"
logs.mkdir(parents=True, exist_ok=True)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(128 + signal.SIGTERM))

mutants = [
    ("immediate_publication", "crates/horizon-client/src/game/modding/block_entity_tick_items.rs",
     "        let key = self.block_entity_tick_key?;",
     "        let key = self.block_entity_tick_key?;\n        if let Some(entity) = self.dropped.iter_mut().find(|e| e.id == id) { entity.stack = replacement.clone().unwrap_or_else(|| ItemStack::new(\"minecraft:air\", 0)); }",
     "native_tick_publishes_or_rejects_item_proposals_with_writeback", {"ROW4_NATIVE_CASE": "invalid_stack"}),
    ("skip_original_stack_check", "crates/horizon-client/src/game/modding/block_entity_tick_items.rs",
     "e.id == proposal.id && e.stack == proposal.original", "e.id == proposal.id",
     "finalization_never_retargets_stale_items_or_replaced_placeholders", {"ROW4_PRECONDITION_CASE": "stack", "HORIZON_LIVE_HOLDER_GAME_CHILD": "valid"}),
    ("drop_decode_poison", "crates/horizon-mod-wasm/src/runtime.rs",
     "horizon_mod_runtime::dispatch_failures::record_dispatch_failure();", "// mutant: discard decode-failure poison",
     "native_tick_publishes_or_rejects_item_proposals_with_writeback", {"ROW4_NATIVE_CASE": "wasm_malformed"}),
]
for name, relative, old, new, test, case in mutants:
    path = ROOT / relative
    original = path.read_text()
    assert original.count(old) == 1, (name, "mutation anchor changed")
    (logs / (name + ".original")).write_text(original)
    try:
        path.write_text(original.replace(old, new))
        env = dict(os.environ, CARGO_INCREMENTAL="0")
        command = ["cargo", "test", "--profile", "fast", "-p", "horizon-client", "--lib", "--no-run", "--message-format=json", "-j", "2"]
        build = subprocess.run(command, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (logs / (name + "-build.log")).write_text(build.stdout)
        assert build.returncode == 0, (name, "mutant must compile")
        executable = None
        for line in build.stdout.splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("reason") == "compiler-artifact" and record.get("executable") and record["target"]["name"] == "horizon_client":
                    executable = record["executable"]
        assert executable, "missing test executable"
        with tempfile.TemporaryDirectory(prefix="row4-mutant-") as home:
            env.update(case)
            homes = {key: str(Path(home) / key) for key in ["HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME", "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"]}
            env = lab.game_env(env, HORIZON_TEST_RESOURCES=str(resources), HORIZON_HOME=home, **homes)
            result = subprocess.run([executable, test, "--ignored", "--nocapture", "--test-threads=1"], cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            (logs / (name + "-test.log")).write_text(result.stdout)
            assert result.returncode == 101 and "assertion" in result.stdout and "FAILED" in result.stdout, (name, "mutant survived or failed for a non-assertion reason")
        print(name + ": killed by " + test, flush=True)
    finally:
        path.write_text(original)
print("All three mutants killed; sources restored. Rebuild before further checks.", flush=True)
