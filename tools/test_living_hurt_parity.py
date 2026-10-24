#!/usr/bin/env python3
"""Compare registered living hurt and synchronized absorption with vanilla."""
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
    def bits(v):return struct.pack('>f',v).hex()
    def op(damage=8,**kw):
        o=dict(damage=bits(damage),blocked=bits(0),fail=-1,game_time=123456789,hit_cooldown=3,hit_last=bits(99),hit_duration=2,hit_time=1,effects=0,**{k:False for k in ['invulnerable','dead_before','dead_after','fire','fire_resistance','sleeping','component','freezing','freeze_extra','damages_helmet','helmet','bypasses_cooldown','no_impact','no_knockback','bypasses_invulnerability','mutate_hit']});o.update(kw);return o
    def row(ops,**kw):
        f=dict(ops=ops,no_action=83,cooldown=0,last_hurt=bits(0),duration=7,time=9,stamp=-3,source=False,player=False);f.update(kw);return f
    values=[-float('inf'),-1e30,-1,-0.,0.,1e-40,.1,1,5,10,20,1e30,float('inf'),float('nan')]
    for v in values:
        for block in values:
            for cooldown in [-2147483648,0,10,11,2147483647]:
                for player in [False,True]:
                    yield row([op(v,blocked=bits(block),effects=2),op(v,blocked=bits(block),component=True),op(v,blocked=bits(block),dead_after=True)],cooldown=cooldown,player=player,last_hurt=bits(v))
    for fail in range(-1,38):
        for cooldown in [0,10,11,20]:
            for dead in [False,True]:
                for component in [False,True]:
                    yield row([op(21,fail=fail,sleeping=True,freezing=True,freeze_extra=True,damages_helmet=True,helmet=True,blocked=bits(2),component=component,dead_after=dead,effects=3,mutate_hit=True),op(22)],player=True,cooldown=cooldown)
    rng=random.Random(0x4855525453455256)
    for _ in range(3000):
        ops=[]
        for __ in range(rng.randrange(1,20)):
            o=op(rng.choice(values),blocked=bits(rng.choice(values)),effects=rng.randrange(4),game_time=rng.choice([-9223372036854775808,-1,0,9223372036854775807]),fail=rng.choice([-1,-1,-1,*range(38)]),hit_last=bits(rng.choice(values)),hit_cooldown=rng.randrange(-30,31))
            for k in ['invulnerable','dead_before','dead_after','fire','fire_resistance','sleeping','component','freezing','freeze_extra','damages_helmet','helmet','bypasses_cooldown','no_impact','no_knockback','bypasses_invulnerability','mutate_hit']:o[k]=rng.randrange(4)==0
            ops.append(o)
        yield row(ops,cooldown=rng.choice([-2147483648,0,10,11,20,2147483647]),last_hurt=bits(rng.choice(values)),source=bool(rng.randrange(2)),player=bool(rng.randrange(2)))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-living-hurt-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaLivingHurtOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/living_hurt_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaLivingHurtOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'living_hurt_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} living hurt output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual LivingEntity.hurtServer invoked directly on base and ServerPlayer receivers. Controlled blocking, inner damage, sounds, knockback, death, attribution and effect callbacks; native empty death-protection lookup and empty-listener victim criterion. Compares field write ordering, cooldown thresholds, float sanitization, modifiers, repeated hits and injected callback failure. Successful totems, attacker-player criteria, actual blocking reduction, combat/death services and full player outer gates remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped living hurt parity passed: {report_path}')


if __name__=='__main__':main()
