#!/usr/bin/env python3
"""Compare level environment reads with both pinned vanilla jars."""
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
    """Decode nonempty JSONL fixture or outcome records in their original order."""
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def main():
    """Compare both authenticated vanilla references and run the public Game regressions."""
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'level-environment-reads'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    fixture = lab.ROOT / 'tests/fixtures/level_environment_reads.jsonl'
    golden = fixture.with_name('level_environment_reads_expected.jsonl')
    source_paths = [Path(__file__), fixture, golden,
        lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLevelEnvironmentReadsOracle.java',
        lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java',
        lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock',
        lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/test_rust_parity.py',
        lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/integration_env.py']
    # Include all Rust inputs, including dynamically included host implementations.
    source_paths += sorted((lab.ROOT / 'crates').rglob('*.rs'))
    hashes = base.source_hashes(source_paths)
    fixed = read_rows(fixture)
    rows = fixed
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    input_hash = lab.digest(inputs, 'sha256')
    classes, _, _ = harness.compile_oracle(build, 'VanillaLevelEnvironmentReadsOracle',
        jars, libraries, lock['minecraft']['java_major'])
    expected = None
    for side in ('server', 'client'):
        output = build / (side + '-expected.jsonl')
        dependencies = os.pathsep.join(map(str, [jars[side], *libraries]))
        harness.java_run(build, classes, dependencies, 'VanillaLevelEnvironmentReadsOracle',
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
    with tempfile.TemporaryDirectory(prefix='horizon-level-reads-') as scratch:
        env = integration_env.game_environment(os.environ, scratch)
        for key in ('XDG_CACHE_HOME', 'HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD',
                    'HORIZON_LAUNCHER_DESKTOP', 'TMPDIR'):
            directory = Path(scratch) / key
            directory.mkdir()
            env[key] = str(directory)
        env.update(HORIZON_TEST_RESOURCES=str(lab.RESOURCES),
                   HORIZON_LEVEL_READ_CASES=str(inputs),
                   HORIZON_LEVEL_READ_EXPECTED=str(build / 'server-expected.jsonl'))
        with (build / 'rust.log').open('w') as output:
            subprocess.run([executable, 'environment_read_tests',
                            '--ignored', '--test-threads=1', '--nocapture'],
                           cwd=scratch, env=env, stdout=output, stderr=subprocess.STDOUT,
                           check=True, timeout=900)
        text = (build / 'rust.log').read_text()
        assert re.search(r'^test result: ok\. 5 passed;', text, re.MULTILINE), text
    base.ensure_sources_unchanged(source_paths, hashes)
    assert lab.digest(inputs, 'sha256') == input_hash, 'Inputs changed during validation'
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'],
        sources_sha256=hashes, input_sha256=input_hash, cases=len(rows),
        fixed_cases=len(fixed), resource_tests=5,
        jars_sha256={side: lab.digest(jars[side], 'sha256') for side in ('server', 'client')},
        expected_sha256=lab.digest(build / 'server-expected.jsonl', 'sha256'),
        rust_test_executable_sha256=lab.digest(Path(executable), 'sha256'),
        scope='Six LevelRef reads through a resource-backed Game against real client/server '
              'biome zoom, closest-biome search, collision floors, dimension metadata and Level '
              'sky update/get calls. Four further Game regressions cover edited cells, parked '
              'levels, client login, stale handles and no resident chunk loading. '
              'Does not run a server tick loop, draw a GUI, or exercise guest transport.'))
    print(f'{len(rows)} level environment cases match both vanilla jars: {report}', flush=True)


if __name__ == '__main__':
    main()
