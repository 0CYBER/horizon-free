#!/usr/bin/env python3
"""Compare native living item-use timers and blocking-item eligibility."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
import math
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    def bits(v): return struct.pack('>f', v).hex()
    modes = ['blocking', 'is-blocking', 'ticks', 'partial']
    ints = [-2147483648, -2147483647, -1, 0, 1, 4, 5, 6, 20, 72000, 2147483646, 2147483647]
    floats = [-float('inf'), -1e30, -1, -.025, -0., 0., 1e-40, .025, .25, 1, 1e30, float('inf'), float('nan')]
    def state(item=0, remaining=71995, virtual_remaining=71995, durations=None, using=True):
        return dict(item=item, remaining=remaining, virtual_remaining=virtual_remaining, durations=durations or [72000, 20], using=using)
    def row(mode='blocking', **kw):
        f = dict(mode=mode, partial=bits(.5), shared_type=False, components=[dict(present=True, delay=bits(.25)), dict(present=True, delay=bits(1))], initial=state(), after=state(item=1, remaining=0, virtual_remaining=-1, durations=[20, 5]), fail=-1, mutate_at=-1)
        f.update(kw)
        return f
    for mode in modes:
        for duration in ints:
            for remaining in ints:
                for item in range(3):
                    yield row(mode, initial=state(item=item, remaining=remaining, virtual_remaining=remaining, durations=[duration, duration]))
        for delay in floats + [i / 40 for i in range(-100, 101)]:
            for elapsed in [-1, 0, 1, 4, 5, 6, 19, 20, 21]:
                yield row(mode, components=[dict(present=True, delay=bits(delay)), dict(present=False, delay=bits(0))], initial=state(remaining=72000-elapsed, virtual_remaining=72000-elapsed), partial=bits(delay))
        for mutate in range(-1, 5):
            for fail in range(-1, 5):
                for item in range(3):
                    for using in [False, True]:
                        yield row(mode, mutate_at=mutate, fail=fail, after=state(item=item, using=using))
    rng = random.Random(0x4954454d555345)
    def number(): return rng.choice(ints) if rng.randrange(2) else rng.randrange(-2147483648, 2147483648)
    def random_state(): return state(rng.randrange(3), number(), number(), [number(), number()], bool(rng.randrange(2)))
    def floating(): return bits(rng.choice(floats)) if rng.randrange(2) else f'{rng.getrandbits(32):08x}'
    for _ in range(12000):
        yield row(rng.choice(modes), partial=floating(), shared_type=bool(rng.randrange(2)), components=[dict(present=bool(rng.randrange(2)), delay=floating()) for _ in range(2)], initial=random_state(), after=random_state(), fail=rng.choice([-1, -1, *range(5)]), mutate_at=rng.randrange(-1, 5))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-use-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemUseOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_use_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemUseOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_use_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item use output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity blocking-item eligibility, isBlocking and both use-timer overloads. Controlled item types supply virtual duration queries; actual use-item/remaining fields and BlocksAttacks components provide captured and reread state. Covers wrapping tick arithmetic, delay boundaries, absent components, empty stacks, shared item types, partial-tick float bits, callback mutations and failures. Item-use lifecycle, durability, codecs and complete combat/world integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item use parity passed: {report_path}')


if __name__=='__main__':main()
