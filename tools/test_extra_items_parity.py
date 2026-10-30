#!/usr/bin/env python3
"""Compare native extra-item deliverys and strict scalar stack validation."""
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
    edges=[-2147483648,-1,0,1,2,64,2147483647]
    def item(count=2,remainder=1,maximum=None,damage=False,cooldown=0x3f800000,group=0):return dict(count=count,remainder=remainder,maximum=maximum,damage=damage,cooldown=cooldown,group=group)
    def config(items=None,**kw):
        v=dict(items=items or [item(),item(1,None,cooldown=None)],finish=0,infinite=False,prototype_max=64,clock=0,extra_count=2,original_count=3,selected=0,slots=[]);v.update(kw);return v
    def row(initial=None,after=None,**kw):
        v=dict(initial=initial or config(),after=after or config([item(1,None,cooldown=None),item(2,None,cooldown=0x40000000,group=1)]),source=0,player=True,fail=-1,mutate=-1);v.update(kw);return v
    for count in edges:
        for finish in range(3):
            for infinite in [False,True]:
                for player in [False,True]:
                    for mutate in [-1,0,1,2,3,4,5]:
                        for fail in [-1,0,1,2,3,4,5]:
                            yield row(config([item(count),item(1,None,cooldown=None)],finish=finish,infinite=infinite),source=0,player=player,mutate=mutate,fail=fail)
    for existing in [0,1,63,64]:
        for remainder in [1,2,64]:
            for occupied in [0,1,35,36]:
                for offhand in [False,True]:
                    for infinite in [False,True]:
                        initial=config([item(2,remainder),item(existing,None,cooldown=None)],slots=[[i,1] for i in range(occupied)]+([[40,1]] if offhand else []))
                        after=config([item(1,remainder),item(existing,None,cooldown=None)],slots=initial['slots'],infinite=infinite)
                        yield row(initial,after,mutate=0)
    rng=random.Random(0x4146544552555345)
    def random_item():return item(rng.choice(edges),rng.choice([None,-1,1,2,64,2147483647]),rng.choice([None,*edges]),bool(rng.randrange(2)),rng.choice([None,0,0x80000000,0x3d4ccccd,0x7f800000,0xff800000,0x7fc00000,rng.randrange(0x100000000)]),rng.randrange(-1,4))
    def random_config():return config([random_item(),random_item()],finish=rng.randrange(3),infinite=bool(rng.randrange(2)),prototype_max=rng.choice(edges),clock=rng.choice(edges),extra_count=rng.choice(edges),original_count=rng.choice(edges))
    for _ in range(2400):
        initial=random_config();after=random_config()
        for c in [initial,after]:
            c['selected']=rng.randrange(9)
            c['slots']=[[i,rng.randrange(3)] for i in rng.sample(range(43),rng.randrange(44))]
        yield row(initial,after,source=rng.randrange(3),player=bool(rng.randrange(2)),mutate=rng.randrange(-1,9),fail=rng.randrange(-1,9))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-extra-items-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaExtraItemsOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/extra_items_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaExtraItemsOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'extra_items_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} extra-item delivery output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual ItemStack.finishUsingItem with validated remainders and cooldowns now invokes inherited ServerPlayer.handleExtraItemsCreatedOnUse and real Inventory/PlayerEquipment insertion. Shared item/component references, occupied/aliased slots, offhand merging, creative material queries, partial insertion and the drop fallback are compared through mutation/failure prefixes. The drop call verifies throwRandomly=false and predicted semantics; its entity-spawn body is controlled. Rust composes item-after-use and inventory pickup. Item handlers, full contained schemas, dropped-entity creation, complete use lifecycle and full gameplay remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped extra-item delivery parity passed: {report_path}')


if __name__=='__main__':main()
