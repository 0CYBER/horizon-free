#!/usr/bin/env python3
"""Compare residency admission order with the real vanilla EntityLookup.

Covers mixed storage families, removal/re-admission and dimension transfers.
Does not claim saved-world chunk loading order parity.
"""
import json
import os
import subprocess

try:
    from . import lab, oracle_harness as h
except ImportError:
    import lab
    import oracle_harness as h


def snapshots(output):
    return [json.loads(line.split('lookup-order: ', 1)[1])
            for line in output.splitlines() if 'lookup-order: ' in line]


def main():
    lock, directory = lab.config()
    manifest, client = lab.import_client(lock, directory)
    libraries = []
    for artifact in lab.client_library_artifacts(manifest):
        if '/natives-' in artifact['path']:
            continue
        libraries.append(lab.import_library(artifact, directory / 'libraries/client'))
    build = lab.LOCAL / 'entity-lookup-order'
    build.mkdir(parents=True, exist_ok=True)
    classes, dependencies, _ = h.compile_oracle(
        build, 'VanillaEntityLookupOrderOracle', {'client': client}, libraries,
        lock['minecraft']['java_major'])
    vanilla = subprocess.check_output([
        'java', '-cp', str(classes) + os.pathsep + dependencies,
        'dev.horizonlab.parity.VanillaEntityLookupOrderOracle'], text=True)
    rust = subprocess.check_output([
        'cargo', 'test', '-p', 'horizon-client-lib', '--lib',
        'lookup_iteration_preserves_insertion_order_not_id_or_family_order',
        '--', '--nocapture'], cwd=lab.ROOT, text=True)
    expected, actual = snapshots(vanilla), snapshots(rust)
    if len(expected) != 6 or actual != expected:
        raise AssertionError(f'vanilla {expected!r} != engine {actual!r}')
    print('6 runtime vanilla lookup snapshots match residency admission order')


if __name__ == '__main__':
    main()
