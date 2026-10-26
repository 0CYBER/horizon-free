#!/usr/bin/env python3
"""Compare JDK map copies and non-comparable keys with captured identity hashes."""
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
    rng=random.Random(0x4944454e54495459)
    def op(kind,k=0,v=0):return [kind,k,v]
    def keys(mode,n):
        keys=[]
        for i in range(n):
            hash=0 if mode==0 else (i%4)*64 if mode==1 else i*31 if mode==2 else rng.randrange(-2**31,2**31)
            keys.append(dict(id=i,kind=rng.randrange(4),hash=hash))
        for i in range(n//4):keys.append(dict(keys[i]))
        return keys
    for mode in range(4):
        for n in [1,2,3,7,8,9,10,11,12,13,24,25,48,49,64,97,128]:
            for order in range(4):
                ks=keys(mode,n);ids=list(range(n))
                if order==1:ids.reverse()
                elif order>1:rng.shuffle(ids)
                ops=[op('put',i,i) for i in ids]+[op('save'),op('copy'),op('cursor')]
                for i in range(n):
                    ops.append(op('next'))
                    if i%2==0:ops.append(op('delete'))
                for i in range(n,len(ks)):ops.extend([op('get',i),op('put',i,-i)])
                ops.extend([op('swap'),op('copy'),op('clear'),op('copy'),op('swap')])
                rng.shuffle(ids)
                ops.extend(op('remove',i) for i in ids)
                yield dict(keys=ks,ops=ops)
    for _ in range(600):
        n=rng.choice([16,32,64,128]);ks=keys(rng.randrange(4),n);ids=list(range(n));rng.shuffle(ids)
        ops=[op('put',i,i) for i in ids]
        for _ in range(200):
            k=rng.choice(['put','put','remove','get','copy','save','swap','cursor','next','force-next','delete','clear'])
            if k=='clear' and rng.randrange(20):k='remove'
            ops.append(op(k,rng.randrange(len(ks)),rng.randrange(-2**31,2**31)))
        yield dict(keys=ks,ops=ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-java-identity-map-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaJavaIdentityMapOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/java_identity_map_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaJavaIdentityMapOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        enriched=build/f'inputs-{side}.txt';enriched_hash=base.source_hashes([enriched])
        run([binary,'java_identity_map_fixture',build,side],'rust-'+side)
        reference=build/f'{side}.txt';rust_output=build/f'rust-{side}.txt';matched=base.compare(reference,rust_output)
        base.ensure_sources_unchanged([enriched],enriched_hash)
        tested_fixtures.update(enriched_hash);fixture_paths.append(enriched)
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'rust_output_sha256':lab.digest(rust_output,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} map copy/identity output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'references':references,'scope':'Actual JDK 25 HashMap with map-copy construction, non-comparable classes, consistently ordered comparable classes and unequal comparable keys that compare as zero. Captures System.identityHashCode in each Java process and supplies those hashes to Rust for the same scenario, then compares linked iteration, copy sizing/reordering, equal-key replacement, tree/list transitions and cursor invalidation. Both game classpath runs use the same installed JDK; this is a collection comparison, not a gameplay or cross-process identity-hash equivalence claim. Key operations are pure and null keys remain outside the API.'}
    lab.write_json(report_path,report);print(f'Scoped Java HashMap parity passed: {report_path}')


if __name__=='__main__':main()
