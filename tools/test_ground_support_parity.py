#!/usr/bin/env python3
"""Compare native supporting-block selection and ground-state transitions."""
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_block_collision_scan_parity as scans
except ImportError:
    import lab
    import test_rust_parity as base
    import test_block_collision_scan_parity as scans


def bits(value):return struct.pack('>d',float(value)).hex()


def selection_corpus():
    for index,f in enumerate(scans.corpus()):
        if f['suffocating']:continue
        v=[struct.unpack('>d',bytes.fromhex(x))[0] for x in f['box']]
        center=[(v[i]+v[i+3])/2 for i in range(3)]
        positions=[center]
        if index%17==0:positions += [[0,0,0],[0.5,0.5,0.5],[float('nan'),0,0],[float('inf'),0,0],[1e154,0,0]]
        for p in positions:yield dict(f,support_position=list(map(bits,p)))
    for shape in [1,3,4,6]:
        for p in [[0,0,0],[0.5,0.5,0.5],[1,1,1],[-0.5,-0.5,-0.5]]:
            for seed in range(6):
                yield dict(box=list(map(bits,[-2,-2,-2,2,2,2])),palette=[[shape,True,True,True],[1,False,False,True]],suffocating=False,missing=0,fail=-1,limit=100000,seed=seed,support_position=list(map(bits,p)))


def state_corpus():
    def initial(flags):return dict(grounded=bool(flags&1),horizontal=bool(flags&2),empty=bool(flags&4),support=[17,-64,-33] if flags&8 else None)
    def op(mode,grounded,horizontal,movement,results,fail,bounds=None):
        return dict(mode=mode,grounded=grounded,horizontal=horizontal,movement=None if movement is None else list(map(bits,movement)),results=results,fail=fail,box=list(map(bits,bounds or [-.3,1,-.3,.3,2.8,.3])))
    for flags in range(16):
        for mode in range(3):
            for grounded in [False,True]:
                for horizontal in [False,True]:
                    for movement in [None,[0,0,0],[.2,-.5,.3]]:
                        for first in [None,[1,2,3]]:
                            for second in [None,[-4,-5,-6]]:
                                for fail in [-1,0,1,2]:yield dict(initial=initial(flags),ops=[op(mode,grounded,horizontal,movement,[first,second],fail)])
    rng=random.Random(0x47524f554e445354)
    points=[None,[0,0,0],[17,-64,-33],[2147483647,-2147483648,2147483647]]
    choices=[-1,-0.0,0,1e-8,.3,1,1e16,float('nan')]
    for _ in range(2000):
        ops=[]
        for _ in range(12):
            low=[rng.choice(choices) for _ in range(3)];high=[v+rng.choice([0,1e-8,1e-6,.6,1.8]) for v in low]
            movement=None if rng.randrange(3)==0 else [rng.choice(choices) for _ in range(3)]
            ops.append(op(rng.randrange(3),bool(rng.randrange(2)),bool(rng.randrange(2)),movement,[rng.choice(points),rng.choice(points)],rng.choice([-1,-1,0,1,2]),[*low,*high]))
        yield dict(initial=initial(rng.randrange(16)),ops=ops)


def main():
    lock,jars,libraries=base.verify_artifacts()
    build=lab.LOCAL/'rust-ground-support-parity';build.mkdir(exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    oracle_dir=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity'
    oracles=[oracle_dir/(name+'.java') for name in ['VanillaBlockCollisionScanOracle','VanillaGroundStateOracle','VanillaFaceSupportOracle']]
    sources=[*oracles,Path(__file__),lab.ROOT/'tools/test_block_collision_scan_parity.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock']
    for crate in ['horizon-core','horizon-world']:sources += [lab.ROOT/'crates'/crate/'Cargo.toml',*sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    names=['block_collision_scan_fixture','ground_state_fixture']
    sources += [lab.ROOT/'crates/horizon-world/examples/probe'/(name+'.rs') for name in names]
    tested=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True);dependencies=os.pathsep.join(map(str,libraries))
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    counts={};fixture_paths=[]
    for name,corpus in [('selection',selection_corpus),('state',state_corpus)]:
        directory=build/name;directory.mkdir(exist_ok=True);path=directory/'inputs.txt';count=0
        with path.open('w') as output:
            for f in corpus():output.write(json.dumps(f,separators=(',',':'))+'\n');count+=1
        counts[name]=count;fixture_paths.append(path)
    fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--locked','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    for group,name in zip(['selection','state'],names):
        binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,name,build/group],'rust-'+group)
    references=[]
    for side,jar in jars.items():
        results=[]
        for group,oracle in zip(['selection','state'],['VanillaBlockCollisionScanOracle','VanillaGroundStateOracle']):
            directory=build/group
            run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),'dev.horizonlab.parity.'+oracle,side,directory],side+'-'+group)
            reference=directory/(side+'.txt');matched=base.compare(reference,directory/'rust.txt')
            results.append(dict(group=group,matched_output_lines=matched,output_sha256=lab.digest(reference,'sha256'),rust_output_sha256=lab.digest(directory/'rust.txt','sha256')))
            print(f'{side}: {matched:,} ground-{group} lines match',flush=True)
        references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),results=results))
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged(fixture_paths,fixtures)
    report=dict(schema=1,minecraft=lock['minecraft']['version'],manifest_sha1=lock['minecraft']['manifest_sha1'],platform=platform.platform(),java=subprocess.check_output(['java','--version'],text=True).splitlines()[0],rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),fixtures=counts,sources_sha256=tested,fixtures_sha256=fixtures,references=references,scope='Actual CollisionGetter.findSupportingBlock over controlled collision metadata/shapes/providers, plus Entity.checkSupportingBlock and both setOnGroundWithMovement overloads over sequential support-query snapshots. Selection compares nearest block-center distance, ties, full scan order and failures. Ground-state checks compare thin foot bounds, previous-horizontal-footprint fallback, null movement, retained/cleared support, flags and partial state on errors across mutation sequences. Registry/context scanning has separate integration coverage; live player grounding, movement lifecycle and gameplay remain pending.')
    lab.write_json(report_path,report);print(f'Scoped ground support parity passed: {report_path}',flush=True)


if __name__=='__main__':main()
