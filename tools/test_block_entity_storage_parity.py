#!/usr/bin/env python3
"""Compare native proto-chunk block-entity storage and NBT coordinate coercion."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab, test_rust_parity as base


def corpus():
    atoms=['none 0','string invalid']
    for kind,values in [('byte',[-128,-1,0,1,127]),('short',[-32768,-1,0,1,32767]),('int',[-2147483648,-17,-16,-1,0,1,15,16,2147483647]),('long',[-9223372036854775808,-4294967297,-2147483649,-1,0,2147483648,4294967297,9223372036854775807])]:
        atoms += [f'{kind} {v}' for v in values]
    atoms += ['float '+bits for bits in ['00000000','80000000','00000001','80000001','3fc00000','bfc00000','4f000000','cf000000','cf000001','7f800000','ff800000','7fc00000','7f800001']]
    atoms += ['double '+bits for bits in ['0000000000000000','8000000000000000','0000000000000001','8000000000000001','3ff8000000000000','bff8000000000000','41e0000000000000','c1e0000000000000','c1e0000000000001','7ff0000000000000','fff0000000000000','7ff8000000000000','7ff0000000000001']]
    for cx,cz in [(0,0),(-1,-1),(1,-2),(-134217728,134217727),(-2147483648,2147483647)]:
        yield f'reset {cx} {cz}'
        for axis in range(3):
            for i,atom in enumerate(atoms):
                xyz=['int -1','int 64','int 31'];xyz[axis]=atom
                yield 'resolve 0 '+' '.join(xyz)
                yield f'pending {i} '+' '.join(xyz)
        yield 'dump'
    for marker in [-9223372036854775808,-1,0,1,9223372036854775807]:
        yield 'reset -1 -1'
        yield f'pending {marker} int 15 int 64 int 15'
        yield 'get -1 64 -1';yield 'save -1 64 -1'
        yield 'mutate -1 64 -1 2147483647'
        yield 'get -1 64 -1';yield 'save -1 64 -1'
        yield f'active -1 64 -1 {marker}'
        yield 'pending 99 int 15 int 64 int 15'
        yield 'get -1 64 -1';yield 'save -1 64 -1';yield 'dump'
        yield 'remove -1 64 -1';yield 'get -1 64 -1';yield 'save -1 64 -1'
    rng=random.Random(0x424c4f434b454e54)
    for trial in range(12):
        cx,cz=trial%3-1,trial%5-2
        yield f'reset {cx} {cz}'
        positions=[(cx*16+x,y,cz*16+z) for x in [0,1,15] for y in [-2147483648,-64,0,64,2147483647] for z in [0,1,15]]
        positions += [(2147483647,-2147483648,-2147483648),(0,4096,0),(67108864,4096,-67108864)]
        for _ in range(500):
            p=rng.choice(positions);xyz=' '.join(map(str,p));op=rng.randrange(7)
            if op<2:yield f'pending {rng.randrange(-2**63,2**63)} int {p[0]} int {p[1]} int {p[2]}'
            elif op==2:yield f'active {xyz} {rng.randrange(-2**63,2**63)}'
            elif op==3:yield f'remove {xyz}'
            elif op==4:yield f'mutate {xyz} {rng.randrange(-2**31,2**31)}'
            elif op==5:yield f'get {xyz}'
            else:yield f'save {xyz}'
            if rng.randrange(8)==0:yield 'dump'
        yield 'dump'
    yield 'reset 0 0'
    for _ in range(20000):
        xyz=[rng.choice([f'float {rng.getrandbits(32):08x}',f'double {rng.getrandbits(64):016x}',rng.choice(atoms)]) for _ in range(3)]
        yield 'resolve 0 '+' '.join(xyz)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-block-entity-storage-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockEntityStorageOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/block_entity_storage_fixture.rs']
    tested_sources=base.source_hashes(sources)
    commands=list(corpus());inputs=build/'inputs.txt';inputs.write_text('\n'.join(commands)+'\n')
    tested_fixtures=base.source_hashes([inputs]);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,oracle],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'block_entity_storage_fixture',inputs,build/'rust.txt'],'rust')
    references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),'dev.horizonlab.parity.VanillaBlockEntityStorageOracle',inputs,build/(side+'.txt')],side)
        matched=base.compare(build/(side+'.txt'),build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} block-entity storage lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged([inputs],tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),
            'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
            'commands':dict(Counter(c.split()[0] for c in commands)),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
            'scope':'Native BlockEntity.getPosFromTag and actual ProtoChunk pending/active insertion, lookup, removal, position union and pending-save fallback. NumericTag coordinate reads cover integer narrowing, float/double floor overflow, infinities/NaNs, missing/nonnumeric fields, wrong-chunk relocation and wrapped chunk coordinates. Active entries replace pending data; later pending records cannot overwrite active entries. In-place pending coordinate mutation retains the original map key. Native chest entities carry loot seeds for active identity and basic saving-metadata comparisons; Rust supplies caller-owned payload serialization. Iteration order is normalized, and pending reads are borrowed/owned values rather than unrestricted Java aliases. Entity creation from NBT, complete entity serializers, active behavior and integration with chunk lifecycle remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped block-entity storage parity passed: {report_path}')

if __name__=='__main__':main()
