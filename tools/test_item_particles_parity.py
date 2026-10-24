#!/usr/bin/env python3
"""Compare inherited eating particle emission with pinned vanilla JARs."""
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
    floats=[0,0x80000000,1,0x3f000000,0x3f800000,0x42b40000,0xc2b40000,0x43b40000,0xc3b40000,0x7f7fffff,0x7f800000,0xff800000,0x7fc00000]
    seeds=[0,1,-1,-9223372036854775808,9223372036854775807,123456789]
    def config(**kw):
        v=dict(position=[1.25,64.5,-2.75],pitch=0,yaw=0,eye=0x3fcf5c29,count=3,damage=7,random=0,level=0,reseed=False,seeds=[99,101]);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(position=[-3.5,71.25,9.5],pitch=0x42b40000,yaw=0xc2b40000,count=0,damage=99,random=1,level=1,reseed=True),seeds=[0,1],item=2,xor=False,particles=3,fail=-1,mutate=-1);v.update(kw);return v
    for xor in [False,True]:
        for seed in seeds:
            for fail in range(-1,15):
                for mutate in range(-1,15):yield row(xor=xor,seeds=[seed,~seed],fail=fail,mutate=mutate)
    for item in range(3):
        for count in [-2147483648,-1,0,1,64,2147483647]:
            for particles in [-2147483648,-1,0,1,5,16]:yield row(item=item,particles=particles,initial=config(count=count))
    for pitch in floats:
        for yaw in floats:yield row(initial=config(pitch=pitch,yaw=yaw),particles=16)
    rng=random.Random(0x5041525449434c45)
    def random_config():return config(position=[rng.choice([-0.0,0.0,1.25,-15.5,29999999.5]) for _ in range(3)],pitch=rng.choice(floats),yaw=rng.choice(floats),eye=rng.choice(floats),count=rng.choice([-1,0,1,16,2147483647]),damage=rng.choice([None,-1,0,99,2147483647]),random=rng.randrange(2),level=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)])
    for _ in range(2400):yield row(initial=random_config(),after=random_config(),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],item=rng.randrange(3),xor=bool(rng.randrange(2)),particles=rng.choice([-1,0,1,3,5,16]),fail=rng.randrange(-1,35),mutate=rng.randrange(-1,35))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-particles-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemParticlesOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_particles_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemParticlesOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_particles_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item particles output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Inherited LivingEntity.spawnItemParticles with real ItemStackTemplate snapshots, LegacyRandomSource and XoroshiroRandomSource. Controlled rotation, eye position, level and particle callbacks compare exact positions/velocities, snapshot identity and values, direct random-field selection, subsequent RNG state and mutation/failure prefixes. Renderer enqueue and full consumable/gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item particles parity passed: {report_path}')


if __name__=='__main__':main()
