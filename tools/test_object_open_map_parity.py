#!/usr/bin/env python3
"""Compare default fastutil Object2ObjectOpenHashMap storage, live indexed entries and callback traversal."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng = random.Random(0x4f50454e4d4150)

    def op(kind, key=-1, value=None):
        return [kind, key, value]

    def hashes(mode, n):
        def unmix(x):
            h = ((x ^ (x >> 16)) * pow(0x9e3779b9, -1, 2**32)) & 0xffffffff
            return h if h < 0x80000000 else h - 2**32
        if mode == 0: return [0] * n
        if mode == 1: return [unmix(31)] * n
        if mode == 2: return [unmix(127 + i % 4) for i in range(n)]
        if mode == 3: return list(range(n))
        return [rng.randrange(-2**31, 2**31) for _ in range(n)]

    def fixture(h, ops, ids=None):
        return dict(hashes=h, ids=list(range(len(h))) if ids is None else ids, ops=ops)

    for mode in range(5):
        for n in [0, 1, 7, 23, 24, 25, 47, 48, 49, 64, 97, 128]:
            for order in range(4):
                h = hashes(mode, n)
                ids = list(range(n))
                rng.shuffle(ids)
                if order % 2: ids.append(-1)
                ops = [op('put', i, None if i % 3 == 0 else i) for i in ids] + [op('cursor')]
                for i in range(len(ids)):
                    ops.extend([op('next'), op('entry-key', i), op('entry-value', i), op('entry-set', i, 700 + i)])
                    if i % 2 == 0 or order >= 2:
                        ops.extend([op('delete'), op('entry-key', i), op('entry-value', i), op('entry-set', i, 800 + i)])
                    if i: ops.extend([op('entry-key', i - 1), op('entry-value', i - 1)])
                ops.extend([op('force-next'), op('delete')])
                for i in reversed(ids):
                    ops.extend([op('contains', i), op('get', i), op('remove', i), op('put', i, -i)])
                rng.shuffle(ids)
                ops.extend(op('remove', i) for i in ids)
                # Old handles retain live indexes even after ordinary remove/clear.
                for i in range(len(ids)):
                    ops.extend([op('entry-key', i), op('entry-set', i, 900 + i), op('entry-value', i)])
                ops.extend(op('get', i) for i in ids)
                ops.extend([op('cursor'), op('clear'), op('next'), op('force-next')])
                yield fixture(h, ops)
    for mode in range(5):
        for n in [1, 8, 24, 25, 49, 128]:
            h = hashes(mode, n)
            yield fixture(h + h, [op('put', i, i) for i in range(n)] +
                          [op('put', i + n, -i) for i in range(n)] +
                          [op('get', i + n) for i in range(n)] +
                          [op('remove', i + n) for i in range(n)], list(range(n)) * 2)
    # Empty-slot values written through stale entries must remain absent to get().
    for null in [False, True]:
        i = -1 if null else 0
        yield fixture([0], [op('put', i, 1), op('cursor'), op('next'), op('remove', i),
                           op('entry-set', 0, 99), op('get', i), op('contains', i),
                           op('clear'), op('entry-value', 0), op('put', i, 2), op('get', i)])
    for _ in range(400):
        n = rng.choice([16, 32, 64, 128])
        h = hashes(rng.randrange(5), n)
        ids = list(range(n)); rng.shuffle(ids)
        ops = [op('put', i, i) for i in ids] + [op('cursor')]
        # Never combine external structural mutation with iterator.remove: an
        # invalidated wrapped key can make the native iterator nonterminating.
        for __ in range(300):
            kind = rng.choice(['put', 'put', 'remove', 'contains', 'get', 'cursor', 'next',
                               'force-next', 'clear', 'entry-key', 'entry-value', 'entry-set'])
            if kind == 'clear' and rng.randrange(20): kind = 'remove'
            k = rng.randrange(20) if kind.startswith('entry-') else rng.randrange(-1, n)
            ops.append(op(kind, k, rng.choice([None, -1, 0, 1, 2**31 - 1, -2**31, rng.randrange(10000)])))
        yield fixture(h, ops)
    for mode in range(5):
        for n in [1, 7, 24, 25, 49, 128]:
            for null in [False, True]:
                h = hashes(mode, 256)
                initial = ([op('put', -1, -1)] if null else []) + [op('put', i, i) for i in range(n)]
                actions = [[], [op('put', 0, 999)], [op('clear')],
                           [op('remove', i) for i in range(n)],
                           [op('put', i, -i) for i in range(n, 256)],
                           [op('clear')] + [op('put', i, i) for i in range(256)],
                           [op('put', -1, 999), op('fail')]]
                for action in actions:
                    for skip in [0, 1, 3]:
                        ops = initial + [['each', [[]] * skip + [action]], ['each', []]]
                        yield fixture(h, ops)
    for _ in range(250):
        n = rng.choice([16, 32, 64, 128])
        h = hashes(rng.randrange(5), n)
        initial = [op('put', i, i) for i in range(rng.randrange(n))]
        if rng.randrange(2): initial.insert(0, op('put', -1, -1))
        actions = []
        for call in range(8):
            actions.append([op(rng.choice(['put', 'put', 'remove', 'clear', 'get', 'fail']),
                               rng.randrange(-1, n), rng.choice([None, call])) for __ in range(rng.randrange(5))])
        yield fixture(h, initial + [['each', actions], ['each', []]])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-object-open-map-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaObjectOpenMapOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/object_open_map_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaObjectOpenMapOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'object_open_map_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} object open map output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'fastutil_sha256':{str(p.relative_to(lab.ROOT)):lab.digest(p,'sha256') for p in libraries if 'fastutil' in str(p)},'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Pinned fastutil 8.5.18 default Object2ObjectOpenHashMap used for effect modifier definitions and attribute instances. Compares put/get/containsKey/remove/clear, null keys/values, retained equal-key identity, slot order, growth/shrink, live indexed entry getKey/getValue/setValue and iterator removal/invalidation including wrapped clusters. forEach exercises captured keys with live values through callback mutation/rehash/failure. Pure hash/equality, nullable Integer values. Alternate constructors/default return values, compute/merge/bulk APIs, entry reuse across different maps and external structural mutation combined with iterator removal remain outside this gate; full attribute/entity gameplay integration is pending.'}
    lab.write_json(report_path,report);print(f'Scoped object open map parity passed: {report_path}')


if __name__=='__main__':main()
