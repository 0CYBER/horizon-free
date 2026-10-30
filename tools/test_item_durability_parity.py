#!/usr/bin/env python3
"""Compare actual native unenchanted item-stack durability mutation."""
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
    modes = ['hurt', 'without', 'set', 'query', 'grow', 'shrink']
    edges = [-2147483648, -2147483647, -1, 0, 1, 2, 19, 20, 21, 16777215, 16777216, 16777217, 2147483646, 2147483647]
    def state(count=1, maximum=20, damage=0, unbreakable=False):
        return dict(count=count, max=maximum, damage=damage, unbreakable=unbreakable)
    def row(mode='hurt', amount=1, **kw):
        f = dict(mode=mode, amount=amount, air=False, player=True, infinite=False, after_infinite=False, initial=state(), after=state(count=2, maximum=5, damage=4), fail=-1, mutate_at=-1)
        f.update(kw)
        return f
    for mode in modes:
        for maximum in [None, *edges]:
            for damage in [None, *edges]:
                for amount in edges:
                    yield row(mode, amount, initial=state(maximum=maximum, damage=damage))
    for mode in ['hurt', 'without']:
        for mutate in range(-1, 5):
            for fail in range(-1, 5):
                for count in [-1, 0, 1, 2]:
                    for maximum in [None, 0, 5, 20]:
                        yield row(mode, 20, mutate_at=mutate, fail=fail, after=state(count=count, maximum=maximum, damage=4))
    rng = random.Random(0x4455524142494c)
    def integer(): return rng.choice(edges) if rng.randrange(2) else rng.randrange(-2147483648, 2147483648)
    def random_state(): return state(rng.choice([-2, 0, 1, 2, 64, 2147483647]), rng.choice([None, integer()]), rng.choice([None, integer()]), bool(rng.randrange(2)))
    for _ in range(10000):
        yield row(rng.choice(modes), integer(), air=bool(rng.randrange(5)==0), player=bool(rng.randrange(2)), infinite=bool(rng.randrange(2)), after_infinite=bool(rng.randrange(2)), initial=random_state(), after=random_state(), fail=rng.choice([-1, -1, *range(5)]), mutate_at=rng.randrange(-1, 5))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-durability-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemDurabilityOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_durability_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemDurabilityOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_durability_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item durability output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual unenchanted ItemStack damage predicates, clamped reads/writes, integer-wrapping count changes, server-level hurtAndBreak and ServerPlayer hurtWithoutBreaking. Real component maps cover absent fields, empty/air stacks, unbreakable and invalid numeric values; callbacks mutate storage and inject failures at infinite-material, empty-listener advancement and break dispatch boundaries. Covers the positive integer-to-float enchantment pipeline with no enchantments. Complete component storage/codecs, enchantment effects, entity equipment break handling and combat/world integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item durability parity passed: {report_path}')


if __name__=='__main__':main()
