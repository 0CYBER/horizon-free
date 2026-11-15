#!/usr/bin/env python3
"""Compare effective ItemResource reads with both pinned vanilla jars."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, integration_env
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, integration_env


def read_rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def boundary_cases():
    for maximum in (1, 3, 7, 31, 400, 16777217, 2147483647):
        for damage in sorted({0, 1, maximum // 2, maximum - 1, maximum,
                              min(maximum + 1, 2147483647)}):
            yield dict(name=f'damage-{maximum}-{damage}', repair='minecraft:iron_ingot',
                       stack=dict(id='minecraft:iron_sword', count=1, components={
                           'minecraft:max_damage': maximum, 'minecraft:damage': damage}))


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'item-resource-properties'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    fixture = lab.ROOT / 'tests/fixtures/item_resource_properties.jsonl'
    golden = fixture.with_name('item_resource_properties_expected.jsonl')
    source_paths = [Path(__file__), fixture, golden,
        lab.ROOT / 'tests/fixture_support/item_resource_properties.rs',
        lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemResourcePropertiesOracle.java',
        lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java',
        lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
        lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
        lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/integration_env.py']
    # Include all Rust inputs, including dynamically included host implementations.
    source_paths += sorted((lab.ROOT / 'crates').rglob('*.rs'))
    hashes = base.source_hashes(source_paths)
    fixed = read_rows(fixture)
    rows = fixed + list(boundary_cases())
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    input_hash = lab.digest(inputs, 'sha256')
    classes, _, _ = harness.compile_oracle(build, 'VanillaItemResourcePropertiesOracle',
        jars, libraries, lock['minecraft']['java_major'])
    expected = None
    for side in ('server', 'client'):
        output = build / (side + '-expected.jsonl')
        dependencies = os.pathsep.join(map(str, [jars[side], *libraries]))
        harness.java_run(build, classes, dependencies, 'VanillaItemResourcePropertiesOracle',
                         [inputs, output], side)
        values = read_rows(output)
        assert len(values) == len(rows), (side, len(values), len(rows))
        assert values[:len(fixed)] == read_rows(golden), f'{side}: committed outcomes differ from vanilla'
        if expected is not None:
            assert values == expected, 'Vanilla client/server outcomes differ'
        expected = values
    # Parse Cargo's artifact stream instead of guessing its target directory or binary hash.
    harness.run_logged(['cargo', 'test', '--locked', '-p', 'horizon-client',
        '--no-default-features', '--lib', '--no-run', '--message-format=json'],
        build, 'compile-rust', timeout=3600)
    artifacts = [json.loads(line) for line in (build / 'compile-rust.log').read_text().splitlines()
                 if line.startswith('{')]
    executables = {record['executable'] for record in artifacts
        if record.get('reason') == 'compiler-artifact' and record.get('executable')
        and record.get('profile', {}).get('test') and record['target']['name'] == 'horizon_client'
        and 'lib' in record['target']['kind']}
    assert len(executables) == 1, executables
    executable = executables.pop()
    with tempfile.TemporaryDirectory(prefix='horizon-item-properties-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        for key in ('XDG_CACHE_HOME', 'HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD',
                    'HORIZON_LAUNCHER_DESKTOP', 'TMPDIR'):
            directory = Path(scratch) / key
            directory.mkdir()
            env[key] = str(directory)
        env.update(HORIZON_TEST_RESOURCES=str(lab.RESOURCES),
                   HORIZON_ITEM_PROPERTIES_CASES=str(inputs),
                   HORIZON_ITEM_PROPERTIES_EXPECTED=str(build / 'server-expected.jsonl'))
        with (build / 'rust.log').open('w') as output:
            subprocess.run([executable, 'item_resource_property_tests',
                            '--ignored', '--test-threads=1', '--nocapture'],
                           cwd=scratch, env=env, stdout=output, stderr=subprocess.STDOUT,
                           check=True, timeout=900)
        text = (build / 'rust.log').read_text()
        assert re.search(r'^test result: ok\. 2 passed;', text, re.MULTILINE), text
    base.ensure_sources_unchanged(source_paths, hashes)
    assert lab.digest(inputs, 'sha256') == input_hash, 'Inputs changed during validation'
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, input_sha256=input_hash, cases=len(rows),
        fixed_cases=len(fixed), boundary_cases=len(rows)-len(fixed),
        jars_sha256={side: lab.digest(jars[side], 'sha256') for side in ('server', 'client')},
        expected_sha256=lab.digest(build / 'server-expected.jsonl', 'sha256'),
        rust_test_executable_sha256=lab.digest(Path(executable), 'sha256'),
        scope='Seven effective ItemResource reads against real client/server ItemStacks with '
              'loaded registry defaults and tags. Rust uses a resource-backed Game and its existing '
              'duration, damageability, glint, rarity and repair consumers; changed-default HUD calculations and '
              'patched mod-item use completion. Bar colors are RGB24. Does not draw a GUI or exercise guest transport.'))
    print(f'{len(rows)} effective item property cases match both vanilla jars: {report}', flush=True)


if __name__ == '__main__':
    main()
