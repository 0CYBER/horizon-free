#!/usr/bin/env python3
"""Compare consumable timing, sound selection and concrete eating particles."""
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
    floats=[0,0x80000000,1,0x3d000000,0x3d4ccccd,0x3f000000,0x3f800000,0x3fcccccd,0xbf800000,0x42b40000,0xc2b40000,0x7f7fffff,0x7f800000,0xff800000,0x7fc00000]
    seeds=[0,1,-1,-9223372036854775808,9223372036854775807,123456789]
    def config(**kw):
        v=dict(position=[1.25,64.5,-2.75],pitch=0,yaw=0,eye=0x3fcf5c29,count=3,damage=7,random=0,level=0,sound=0,reseed=False,seeds=[99,101]);v.update(kw);return v
    def row(**kw):
        v=dict(initial=config(),after=config(position=[-3.5,71.25,9.5],pitch=0x42b40000,yaw=0xc2b40000,count=1,damage=99,random=1,level=1,sound=1,reseed=True),seeds=[0,1],item=2,xor=False,particles=3,fail=-1,mutate=-1,animation=1,seconds=0x3fcccccd,remaining=24,sound=0,has_particles=True,emit_random=0,override=True);v.update(kw);return v
    for xor in [False,True]:
        for seed in seeds:
            for animation in [1,2]:
                for fail in range(-1,12):
                    for mutate in range(-1,12):yield row(xor=xor,seeds=[seed,~seed],animation=animation,fail=fail,mutate=mutate)
    for animation in range(12):
        for override in [False,True]:
            for particles in [False,True]:
                for emit_random in [0,1]:yield row(animation=animation,override=override,has_particles=particles,emit_random=emit_random)
    for seconds in floats:
        for remaining in [-2147483648,-2147483647,-33,-32,-1,0,1,4,7,8,20,24,25,28,32,2147483647]:yield row(seconds=seconds,remaining=remaining,has_particles=False)
    rng=random.Random(0x434f4e53554d45)
    def random_config():return config(position=[rng.choice([-0.0,0.0,1.25,-15.5,29999999.5]) for _ in range(3)],pitch=rng.choice(floats),yaw=rng.choice(floats),eye=rng.choice(floats),count=rng.choice([-1,0,1,16,2147483647]),damage=rng.choice([None,-1,0,99,2147483647]),random=rng.randrange(2),level=rng.randrange(2),sound=rng.randrange(2),reseed=bool(rng.randrange(2)),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)])
    for _ in range(2400):yield row(initial=random_config(),after=random_config(),seeds=[rng.randrange(-2**63,2**63) for _ in range(2)],item=rng.randrange(3),xor=bool(rng.randrange(2)),particles=rng.choice([-1,0,1,3,5,16]),fail=rng.randrange(-1,35),mutate=rng.randrange(-1,35),animation=rng.randrange(12),seconds=rng.randrange(2**32),remaining=rng.randrange(-2**31,2**31),sound=rng.randrange(2),has_particles=bool(rng.randrange(2)),emit_random=rng.randrange(2),override=bool(rng.randrange(2)))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-consumable-emission-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaConsumableEmissionOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/consumable_emission_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaConsumableEmissionOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'consumable_emission_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} consumable emission output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual Consumable timing and emitParticlesAndSounds composed with inherited LivingEntity.spawnItemParticles, real templates and both native RNGs. Compares all animations, unconditional random draws, supplied versus direct random sources, sound overrides after particle callbacks, retained sound parameters, float timing edges and mutation/failure prefixes. Audio playback, rendering, consumable completion/effects and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped consumable emission parity passed: {report_path}')


if __name__=='__main__':main()
