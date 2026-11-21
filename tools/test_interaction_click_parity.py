#!/usr/bin/env python3
"""Compare a server's `Interaction.interact` with Horizon's interaction click.

The Java oracle builds a real server `Interaction` per `response` value, with the
level's game time and the clicker's id pinned, runs the shipped
`Interaction.interact`, and prints the answer and the fields
`Interaction.addAdditionalSaveData` then saves. Horizon runs the same two clicks
through its own click chain, from a guest's interact packet to the box, and prints
the same words. Only the click is compared: `skipAttackInteraction`, a client's own
prediction and the metadata defaults Horizon adds around the box are outside it.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base
    from . import test_model_baking_parity as baking
except ImportError:
    import lab
    import oracle_harness as harness
    import test_rust_parity as base
    import test_model_baking_parity as baking

ORACLE = 'VanillaInteractionClickOracle'
TEST = 'game::interaction_click_parity_tests::a_click_on_a_server_interaction_consumes_like_vanilla'
CASES = ['response_false', 'response_true']


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'interaction-click-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    run = build / 'rust.json'
    run.unlink(missing_ok=True)
    sources = [lab.ROOT / path for path in (
        'tools/test_interaction_click_parity.py',
        'tools/oracle_harness.py',
        'tools/test_rust_parity.py',
        'tools/lab.py',
        'crates/horizon-client/src/game/items/item_actions.rs',
        'crates/horizon-client/src/game/items/interactions.rs',
        'crates/horizon-client/src/game/items/interaction_click_parity_tests.rs',
        f'mods/horizon/src/test/java/dev/horizonlab/parity/{ORACLE}.java')]
    hashes = base.source_hashes(sources)
    classes, dependencies, _ = harness.compile_oracle(
        build, ORACLE, jars, baking.client_libraries(), lock['minecraft']['java_major'])
    harness.java_run(build, classes, dependencies, ORACLE, [build / 'java.json'], 'java')
    # The probe's trace goes to a scratch folder: it is one more output of the run, not a
    # build artifact, and a test binary's own writes are the run's, not the driver's.
    with tempfile.TemporaryDirectory(prefix='interaction-click-') as scratch:
        trace = Path(scratch) / 'rust.json'
        env = lab.game_env(HORIZON_TEST_RESOURCES=args.resources.resolve(),
                           HORIZON_INTERACTION_CLICK_TRACE=trace)
        with (build / 'rust.log').open('w') as log:
            subprocess.run(
                ['cargo', 'test', '--locked', '--profile', 'fast', '-p', 'horizon-client', '--lib',
                 TEST, '--', '--ignored', '--exact'],
                cwd=lab.ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                check=True, timeout=3600)
        if not trace.is_file():
            raise AssertionError(f'the Rust probe wrote no trace: {build / "rust.log"}')
        run.write_bytes(trace.read_bytes())
    vanilla = json.loads((build / 'java.json').read_text())
    rust = json.loads(run.read_text())
    covered = [row['case'] for row in vanilla]
    if covered != CASES:
        raise AssertionError(f'vanilla oracle covers {covered}, not {CASES}')
    if rust != vanilla:
        raise AssertionError(f'the interaction click differs: vanilla {vanilla}; Horizon {rust}')
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], cases=len(vanilla),
        sources_sha256=hashes, client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_sha256=lab.digest(build / 'java.json', 'sha256'),
        rust_sha256=lab.digest(run, 'sha256'), scope=__doc__))
    print(f'{len(vanilla)} interaction clicks match vanilla, `response` false and true: {report}')


if __name__ == '__main__':
    main()