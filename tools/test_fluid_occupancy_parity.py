#!/usr/bin/env python3
"""Compare section-based liquid occupancy and registered fluid presence."""
import itertools
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(info):
    d=info['defaults'];air=d['minecraft:air'];water=d['minecraft:water'];stone=d['minecraft:stone']
    def bits(v):return struct.pack('>d',float(v)).hex()
    def op(bounds=(0,0,0,1,1,1),writes=(),fail=-1,compact=False):return dict(bounds=[bits(v) for v in bounds],writes=list(writes),fail=fail,compact=compact)
    def row(base=air,min=-16,height=48,ops=None):return dict(base=base,min=min,height=height,ops=ops or [op()])
    boxes=[(0,0,0,1,1,1),(-.5,-.5,-.5,.5,.5,.5),(0,0,0,0,0,0),(15.5,-.1,15.5,16.5,16.1,16.5),(-16,-16,-16,16,32,16),(0,-32,0,1,-17,1),(0,32,0,1,64,1),(0,-32,0,1,64,1),(16,16,16,15,15,15),(30000000,-64,-30000000,30000001,1,-29999999)]
    bases=[d['minecraft:'+n] for n in ['air','stone','water','lava','bubble_column','kelp','seagrass','oak_slab','oak_stairs','mangrove_roots','water_cauldron','powder_snow']]
    for base in bases:
        for box in boxes:
            for fail in range(-1,20):yield row(base=base,ops=[op(box,fail=fail)])
    positions=[(0,0,0),(1,1,1),(15,15,15),(0,-16,0),(15,31,15),(0,16,0)]
    for p in positions:
        for box in boxes:
            yield row(ops=[op(box,[(*p,water)]),op(box,[(*p,stone)]),op(box,[(*p,air)],compact=True),op(box,[(*p,water)])])
    for state in range(0,info['states'],97):yield row(base=state,ops=[op(boxes[1]),op(boxes[4])])
    rng=random.Random(0x4c4951554944)
    for _ in range(1400):
        minimum=rng.choice([-64,-16,0,16]);height=rng.choice([16,32,64]);ops=[]
        for _ in range(4):
            y=rng.randrange(minimum,minimum+height);box=rng.choice(boxes)
            ops.append(op(box,[(rng.randrange(16),y,rng.randrange(16),rng.randrange(info['states'])) for _ in range(rng.randrange(12))],fail=rng.choice([-1,-1,*range(20)]),compact=bool(rng.randrange(2))))
        yield row(base=rng.choice(bases),min=minimum,height=height,ops=ops)


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-fluid-occupancy-parity';build.mkdir(exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    root=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[root/(n+'.java') for n in ['VanillaFluidOccupancyOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/test_worldgen_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/examples/probe/fluid_occupancy_fixture.rs']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    def java(side,mode):run(['java','-Xmx768M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependencies]),'dev.horizonlab.parity.VanillaFluidOccupancyOracle',mode,build],side+'-'+mode)
    java('server','export');info=json.loads((build/'info.json').read_text());count=0
    if info['states']!=35723:raise AssertionError('Incomplete pinned registry')
    with (build/'inputs.txt').open('w') as output:
        for f in corpus(info):output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
    fixture_paths=[build/n for n in ['inputs.txt','info.json','block-registry.nbt']];fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust');metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'fluid_occupancy_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side);matched=base.compare(build/(side+'.txt'),build/'rust.txt');references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(build/(side+'.txt'),'sha256'),matched_output_lines=matched));print(f'{side}: {matched:,} fluid-occupancy output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),fixtures=count,registry_states=info['states'],sources_sha256=tested,fixtures_sha256=fixtures,rust_output_sha256=lab.digest(build/'rust.txt','sha256'),references=references,scope='Actual LevelReader.containsAnyLiquid and native BoxBlockMatcher/BlockScanUtils over real ProtoChunk sections and palettes. All registered block fluid-presence values, inclusive AABB floors, chunk/section scan order, height limits, palette changes/compaction, nonfinite rejection and explicit Rust query budgets. Public query traces and results match; Rust skips dry active palettes without unpacking cells. Controlled repeating resident chunk snapshots; Entity.isFree collision composition and live world integration remain pending.')
    lab.write_json(report_path,report);print(f'Scoped fluid occupancy parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
