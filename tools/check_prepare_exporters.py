#!/usr/bin/env python3
"""Compare two isolated preparations with a batched, runtime-only release preparation.

Uses only locally imported pinned Minecraft files. Run after compile-exporters and copy its
server/, client/ and manifest.json to exporters/. Byte differences are accepted only where
both isolated runs also differ and normalize identically (known exporter noise).
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def hashes(folder):
    return {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in folder.rglob('*') if p.is_file() and p.suffix != '.log'
            and not {'classes', 'client-classes'} & set(p.relative_to(folder).parts)
            and p.name != 'horizon-resources.json'}


def normalize(path):
    value = json.loads(path.read_text())
    if path.name == 'block-behaviours.json':
        # JVM-assigned lambda class addresses are printed as opaque field values.
        def clean(node):
            if isinstance(node, str):
                return re.sub(r'\$\$Lambda/0x[0-9a-fA-F]+', '$$Lambda/<address>', node)
            if isinstance(node, list):
                return [clean(item) for item in node]
            if isinstance(node, dict):
                return {key: clean(item) for key, item in node.items()}
            return node
        value = clean(value)
    elif path.name == 'creative-inventory.json':
        for tab in value['tabs']:
            if 'tags' in tab:
                tab['tags'] = [sorted(tags) for tags in tab['tags']]
    return value


def verify(folders):
    a, b, batched = [hashes(folder) for folder in folders]
    if a.keys() != b.keys() or a.keys() != batched.keys():
        raise SystemExit('Resource file sets differ')
    known = {'render-definitions.json', 'block-behaviours.json', 'creative-inventory.json'}
    noise = [name for name in a if a[name] != b[name] or a[name] != batched[name]]
    differences = []
    for name in noise:
        if name not in known:
            differences.append(name)
        else:
            x, y, z = [normalize(folder / name) for folder in folders]
            if x != y or x != z:
                differences.append(name)
    manifests = []
    for folder, raw in zip(folders, [a, b, batched]):
        replacements = {raw[name]: f'normalized:{name}' for name in noise}
        def clean(node):
            if isinstance(node, str):
                return replacements.get(node, node)
            if isinstance(node, list):
                return [clean(item) for item in node]
            if isinstance(node, dict):
                return {key: clean(item) for key, item in node.items()}
            return node
        manifests.append(clean(json.loads((folder / 'horizon-resources.json').read_text())))
    if manifests[0] != manifests[1] or manifests[0] != manifests[2]:
        differences.append('horizon-resources.json')
    print(f'Normalized exporter noise: {noise}')
    if differences:
        raise SystemExit(f'Resources differ: {differences}')
    print(f'PASS: all {len(a)} resource files match, with only known noise normalized')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--work', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='prepare-exporters-') as temporary:
        work = args.work.resolve() if args.work else Path(temporary)
        work.mkdir(parents=True, exist_ok=True)
        # Freeze the tested executable even if a developer rebuilds concurrently.
        frozen = work / binary.name
        shutil.copy2(binary, frozen)
        binary = frozen
        results = []
        times = []
        for name in ['isolated-1', 'isolated-2', 'batched']:
            folder = work / name
            folder.mkdir()
            shutil.copyfile(ROOT / 'research-lock.json', folder / 'research-lock.json')
            shutil.copytree(ROOT / 'mods/horizon/src/test/java', folder / 'mods/horizon/src/test/java')
            shutil.copytree(ROOT / 'exporters', folder / 'exporters')
            (folder / '.local').mkdir()
            (folder / '.local/minecraft').symlink_to((ROOT / '.local/minecraft').resolve(), target_is_directory=True)
            env = dict(os.environ)
            env.pop('HORIZON_PREPARE_ISOLATED', None)
            if name.startswith('isolated'):
                env['HORIZON_PREPARE_ISOLATED'] = '1'
            else:
                # An actual runtime-only home: no javac in bin. Java resolves its own libraries.
                java = Path(shutil.which('java')).resolve()
                runtime = folder / 'runtime'
                (runtime / 'bin').mkdir(parents=True)
                trace = folder / 'java-invocations.txt'
                wrapper = runtime / 'bin' / java.name
                wrapper.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> '
                                   + shlex.quote(str(trace)) + '\nexec '
                                   + shlex.quote(str(java)) + ' "$@"\n')
                wrapper.chmod(0o755)
                env['JAVA_HOME'] = str(runtime)
                env['PATH'] = str(runtime / 'bin')
            started = time.monotonic()
            with (work / f'{name}.log').open('w') as log:
                subprocess.run([str(binary), '--root', str(folder)], cwd=folder, env=env,
                               stdout=log, stderr=subprocess.STDOUT, check=True, timeout=1200)
            if name == 'batched':
                launches = [line for line in trace.read_text().splitlines() if line != '-version']
                if len(launches) != 5:
                    raise SystemExit(f'Expected 5 exporter/data-generator JVMs, got {len(launches)}')
                print(f'Runtime-only preparation: {len(launches)} JVM launches', flush=True)
            text = (work / f'{name}.log').read_text()
            if 'Compiling ' in text:
                raise SystemExit(f'{name} unexpectedly needed a compiler')
            resources = next((folder / '.local/horizon-client').iterdir())
            results.append(resources)
            times.append(round(time.monotonic() - started, 2))
            print(f'{name}: {times[-1]}s, prepared', flush=True)
        verify(results)
        print(f'Isolated {times[:2]}s, batched {times[2]}s')


if __name__ == '__main__':
    main()
