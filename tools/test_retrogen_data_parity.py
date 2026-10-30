#!/usr/bin/env python3
"""Compare native below-zero retrogen NBT, bedrock-hole masks and queries."""
import argparse
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
    targets=[['string',name] for name in ['structure_starts',':structure_references','minecraft:biomes','terrain','features','initialize_light','light','spawn','full','empty','unknown','horizon:full']]+[None,['int',9],['compound',{}]]
    masks=[None,['long_array',[]],['long_array',[0]*8],['long_array',[-1]*4],['long_array',[1,0,0,0,0,1,0]],['byte_array',[-128,-1,0,1,127]],['int_array',[-2147483648,-1,0,1,2147483647]],['list',[]],['list',[['byte',-128],['short',-32768],['int',-2147483648],['long',-9223372036854775808]]],['list',[['double','bff8000000000000'],['float','3fc00000'],['double','7ff8000000000000'],['float','ff800000']]],['list',[['long',1],['string','invalid']]],['string','invalid'],['int',1],['compound',{}]]
    for bit in [*range(256),256,511,512,1023]:
        words=[0]*(bit//64+1);word=1<<(bit%64);words[-1]=word if word<2**63 else word-2**64;masks.append(['long_array',words])
    for target in targets:
        for mask in masks:
            fields={}
            if target is not None:fields['target_status']=target
            if mask is not None:fields['missing_bedrock']=mask
            yield ['compound',fields]
    rng=random.Random(0x524554524f47454e)
    for _ in range(1000):
        words=[rng.randrange(-2**63,2**63) for _ in range(rng.randrange(17))]+[0]*rng.randrange(4)
        yield ['compound',{'target_status':['string','full'],'missing_bedrock':['long_array',words],'extra':['string','ignored']}]
    for kind,value in [('end',None),('byte',1),('string','full'),('list',[])]:yield [kind,value]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-retrogen-data-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaRetrogenDataOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/retrogen_data_fixture.rs']
    tested_sources=base.source_hashes(sources)
    commands=list(corpus());inputs=build/'inputs.txt';inputs.write_text('\n'.join(json.dumps(command) for command in commands)+'\n')
    tested_fixtures=base.source_hashes([inputs]);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,oracle],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'retrogen_data_fixture',inputs,build/'rust.txt'],'rust')
    references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),'dev.horizonlab.parity.VanillaRetrogenDataOracle',inputs,build/(side+'.txt')],side)
        matched=base.compare(build/(side+'.txt'),build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} retrogen-data lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged([inputs],tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),
            'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
            'codec_cases':len(commands),'hole_queries_per_decoded_input':1229,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
            'scope':'Native BelowZeroRetrogen codec, non-empty target-status validation, lenient optional bedrock mask decoding, NumericTag/array/list numeric streams, canonical trailing-zero removal, omitted empty masks, hasBedrockHoles and wrapped local X/Z hole queries. All 256 column bits and extra bits beyond the chunk footprint, malformed target/mask fields, mixed numeric lists and random bitsets are covered. Codec error text is normalized rather than compared. Generation scheduling, status transition cleanup, bedrock replacement/masking, biome retention and chunk lifecycle integration remain outside this data comparison.'}
    lab.write_json(report_path,report);print(f'Scoped retrogen-data parity passed: {report_path}')

if __name__=='__main__':main()
