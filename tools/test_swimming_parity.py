#!/usr/bin/env python3
"""Compare Entity/Player swimming state transitions and water predicates with vanilla."""
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
    def row(mask=0,**kw):
        f={k:bool(mask&(1<<i)) for i,k in enumerate(['swimming','water','sprint','passenger','eye','flying'])}
        f.update(mode='update',player=False,fluid=2,water_mask=6,eye_height=struct.pack('>f',1.62).hex(),fail=-1,mutate_at=-1);f.update(kw);return f
    for player in [False,True]:
        for mask in range(64):
            for mode in ['update','under','shallow']:
                for fluid in range(5):
                    for tag in [0,6,31]:yield row(mask,player=player,mode=mode,fluid=fluid,water_mask=tag)
            for mode in ['update','under','shallow']:
                for fail in range(-1,8):
                    for mutate_at in range(-1,8):yield row(mask,player=player,mode=mode,fail=fail,mutate_at=mutate_at)
    for height in [-float('inf'),-1,-0.,0.,.39999998,.4,.40000004,1,float('inf'),float('nan')]:yield row(mode='threshold',eye_height=struct.pack('>f',height).hex())
    rng=random.Random(0x5357494d)
    for _ in range(3000):yield row(mode='threshold',eye_height=f'{rng.getrandbits(32):08x}')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-swimming-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSwimmingOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/swimming_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaSwimmingOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'swimming_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} swimming output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Entity and Player updateSwimming, isUnderWater, isInShallowWater and fluid jump threshold. Compares previous-eye/body flags, player direct flying ability guard, sprint/passenger branches, direct block-position capture, native fluid tag membership, callbacks mutating fields and injected failures. Entity data and cell fluids are controlled; full synchronized entity storage, boat interaction boxes and playable gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped swimming parity passed: {report_path}')


if __name__=='__main__':main()
