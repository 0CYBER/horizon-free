#!/usr/bin/env python3
"""Check a visible primed TNT's crosshair pick against the pinned LocalPlayer.pick."""
import argparse
import os
from pathlib import Path
import subprocess

try:
    from . import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base
except ImportError:
    import lab
    import oracle_harness as harness
    import test_model_baking_parity as baking
    import test_rust_parity as base

ORACLE = 'VanillaPrimedTntCrosshairOracle'
TEST = ('game::remote::interaction_event_tests::primed_tnt_crosshair_tests::'
        'the_crosshair_picks_only_visible_in_reach_primed_tnt')
PREFIX = 'tnt-crosshair: '
CASES = ('open', 'wall', 'nearer_cow', 'farther_cow', 'out_of_reach', 'removed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path,
                        default=Path(os.environ.get('HORIZON_TEST_RESOURCES', lab.RESOURCES)))
    parser.add_argument('--output', type=Path, default=lab.LOCAL / 'primed-tnt-crosshair-parity')
    args = parser.parse_args()
    build = args.output.resolve()
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    lock, jars, _ = base.verify_artifacts()
    classes, dependencies, oracle = harness.compile_oracle(
        build, ORACLE, jars, baking.client_libraries(), lock['minecraft']['java_major'])
    sources = [Path(__file__), oracle, *(
        lab.ROOT / 'crates/horizon-client/src/game' / source for source in [
            'network/remote_vehicles.rs', 'network/remote/interaction_actions.rs',
            'network/remote/interaction_prediction.rs', 'network/remote/primed_tnt_crosshair_tests.rs',
            'entities/tnt.rs', 'items/hand.rs'
        ]
    )]
    hashes = base.source_hashes(sources)
    expected = build / 'client.txt'
    harness.java_run(build, classes, dependencies, ORACLE, [expected], 'java')

    with (build / 'rust.log').open('w', encoding='utf-8') as log:
        subprocess.run(['cargo', 'test', '--locked', '-p', 'horizon-client', '--lib',
                        TEST, '--', '--exact', '--ignored', '--nocapture'],
                       cwd=lab.ROOT, env=lab.game_env(HORIZON_TEST_RESOURCES=args.resources.resolve()),
                       stdout=log, stderr=subprocess.STDOUT, check=True, timeout=3600)
    output = (build / 'rust.log').read_text(encoding='utf-8')
    lines = [line.removeprefix(PREFIX) for line in output.splitlines() if line.startswith(PREFIX)]
    actual = build / 'rust.txt'
    actual.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    if tuple(line.split()[0] for line in expected.read_text(encoding='utf-8').splitlines()) != CASES:
        raise AssertionError('Vanilla oracle did not cover every pick case')
    matched = harness.compare(expected, actual, len(CASES))
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], cases=matched,
        sources_sha256=hashes, client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        java_sha256=lab.digest(expected, 'sha256'), rust_sha256=lab.digest(actual, 'sha256'),
        scope=__doc__))
    print(f'{matched} primed TNT crosshair picks match the pinned client: {report}')


if __name__ == '__main__':
    main()
