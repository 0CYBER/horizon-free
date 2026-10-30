#!/usr/bin/env python3
"""Compare pinned chunk-status ordering, metadata, lookup and NBT codecs."""
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
    names=['empty','structure_starts','structure_references','biomes','terrain','features','initialize_light','light','spawn','full']
    for name in names:
        for prefix in ['',':','minecraft:','horizon:']:
            yield ['string',prefix+name]
    for name in ['',':','minecraft:','EMPTY','minecraft:Full','minecraft:noise','minecraft:surface','minecraft:carvers','full ', ' full', 'minecraft::full', 'minecraft/full','FULL','horizon:full','\u0000','\U0001f30d']:
        yield ['string',name]
    rng=random.Random(0x535441545553)
    alphabet='abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_:/. -\t\n'
    for _ in range(2000):yield ['string',''.join(rng.choices(alphabet,k=rng.randrange(1,40)))]
    for value in [-2147483648,-1,0,1,9,10,2147483647]:yield ['int',value]
    yield ['list',None];yield ['compound',None]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-chunk-status-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaChunkStatusOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/chunk_status_fixture.rs']
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
    run([binary,'chunk_status_fixture',inputs,build/'rust.txt'],'rust')
    references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),'dev.horizonlab.parity.VanillaChunkStatusOracle',inputs,build/(side+'.txt')],side)
        matched=base.compare(build/(side+'.txt'),build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(build/(side+'.txt'),'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} chunk-status lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged([inputs],tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),
            'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
            'statuses':10,'ordered_pairs':100,'codec_cases':len(commands),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,
            'scope':'Actual pinned ChunkStatus status list, ordering and maximum for all pairs, parent identity, chunk kind, selected heightmaps, direct byName fallback and NBT codec acceptance/encoding. All registered names cover qualified, unqualified and empty-namespace forms; unknown, legacy, malformed, random and nonstring inputs are checked. Codec error text is normalized rather than compared. Rust pre-light chunk metadata uses these same status definitions; this does not execute generation steps, implement lighting, or establish full chunk lifecycle parity.'}
    lab.write_json(report_path,report);print(f'Scoped chunk-status parity passed: {report_path}')

if __name__=='__main__':main()
