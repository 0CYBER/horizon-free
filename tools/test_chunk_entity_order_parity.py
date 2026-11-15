#!/usr/bin/env python3
"""Compare chunk entity lookup and save order with the real vanilla manager.

Mixed storage families loaded with two chunks, one chunk's unload (its stored
roots) and reload, a dimension round trip and a save, against
PersistentEntitySectionManager and its EntityLookup.
"""
import json
import os
import subprocess
import sys

try:
    from . import lab, oracle_harness as h
except ImportError:
    import lab
    import oracle_harness as h

TEST = 'chunk_loaded_families_keep_vanilla_lookup_and_save_order'


def lines(output, label):
    return [json.loads(line.split(label, 1)[1])
            for line in output.splitlines() if label in line]


def observed(output):
    # A save writes its chunks in no particular order: compare them by chunk.
    saves = lines(output, 'saved-order: ')
    unload, final = saves[:1], sorted(saves[1:])
    return {'lookup': lines(output, 'load-order: '), 'unload': unload, 'saved': final}


def vanilla():
    lock, directory = lab.config()
    manifest, client = lab.import_client(lock, directory)
    libraries = []
    for artifact in lab.client_library_artifacts(manifest):
        if '/natives-' in artifact['path']:
            continue
        libraries.append(lab.import_library(artifact, directory / 'libraries/client'))
    build = lab.LOCAL / 'chunk-entity-order'
    build.mkdir(parents=True, exist_ok=True)
    classes, dependencies, _ = h.compile_oracle(
        build, 'VanillaChunkEntityOrderOracle', {'client': client}, libraries,
        lock['minecraft']['java_major'])
    return subprocess.check_output([
        'java', '-cp', str(classes) + os.pathsep + dependencies,
        'dev.horizonlab.parity.VanillaChunkEntityOrderOracle'], text=True)


def engine():
    resources = os.environ.get('HORIZON_TEST_RESOURCES') or str(
        lab.LOCAL / 'horizon-client' / '26.3')
    return subprocess.check_output([
        'cargo', 'test', '-p', 'horizon-client', '--lib', TEST,
        '--', '--ignored', '--exact', f'game::persist::lookup_order_tests::{TEST}',
        '--nocapture'],
        cwd=lab.ROOT, text=True, env={**os.environ, 'HORIZON_TEST_RESOURCES': resources})


def main():
    expected = observed(vanilla())
    if '--oracle' in sys.argv:
        print(json.dumps(expected))
        return
    actual = observed(engine())
    if len(expected['lookup']) != 5 or actual != expected:
        raise AssertionError(f'vanilla {expected!r} != engine {actual!r}')
    print('5 lookup snapshots, the unload store and the save match vanilla chunk entity order')


if __name__ == '__main__':
    main()
