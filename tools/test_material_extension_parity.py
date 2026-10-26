#!/usr/bin/env python3
"""Compare badlands and iceberg column extensions with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import struct
import subprocess
try:
    from . import lab,test_rust_parity as base,test_worldgen_parity as worldgen,test_biome_temperature_parity as biome
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_biome_temperature_parity as biome


def corpus(biomes):
    def column(pattern):
        values=[]
        for y in range(-64,192):
            if pattern==0:state=3 if y<32 else 4 if y<63 else 0
            elif pattern==1:state=9 if y<32 else 5 if y<63 else y%3
            elif pattern==2:state=11 if y<0 else y%3
            elif pattern==3:state=8 if y<0 else 10 if y<63 else 12 if y==63 else 0
            else:state=y%3
            values.append(state)
        return values
    positions=[(0,0),(17,-31),(-1000,3001),(2**24+3,-2**24-3),(137,911),(-16,16)]
    for i,name in enumerate(sorted(biomes)):
        for legacy in [False,True]:
            for seed in [0,-1]:
                ops=[]
                for j,(x,z) in enumerate(positions):ops += [['badlands',x,z,64,-64],['frozen',x,z,64,[-64,0,50,64][j%4]]]
                yield {'seed':seed,'legacy':legacy,'sea':63,'default':3,'biome':name,'min':-64,'initial':column(0),'limit':4096,'ops':ops}
    bits=lambda value:f'{struct.unpack(">I",struct.pack(">f",value))[0]:x}'
    triples=[['0','3f800000','0'],['3f800000','bf800000','3f800000'],['3f800000','0','3f800000'],['3f000000','3f000000','0'],['3f800000','3f800000','3f800000'],['7fc00000','3f800000','3f800000'],['3f800000','7fc00000','3f800000'],['3f800000','3f800000','7fc00000'],['7f800000','7f800000','0'],['7f800000','7f800000','7f800000'],['80000000','80000000','80000000']]
    threshold=struct.unpack('>I',struct.pack('>f',1.8/15))[0]
    triples += [[bits(1.0),f'{threshold+step:x}',bits(1.0)] for step in [-1,0,1]]
    for i,triple in enumerate(triples):
        for pattern in range(5):
            for legacy in [False,True]:
                name=sorted(biomes)[(i*5+pattern)%len(biomes)]
                ops=[['badlands',17,-31,64,-64],['badlands',17,-31,64,-64],['frozen',17,-31,64,-64],['frozen',17,-31,64,-64],['frozen',-1000,3001,100,50]]
                yield {'seed':-2**63 if legacy else 2**63-1,'legacy':legacy,'sea':[63,-17,127][pattern%3],'default':[3,8,3,8,4][pattern],'biome':name,'min':-64,'initial':column(pattern),'noise_bits':triple+triple,'limit':4096,'ops':ops}
    for sea in [-2**31,2**31-1]:
        yield {'seed':0,'legacy':False,'sea':sea,'default':3,'biome':sorted(biomes)[0],'min':-64,'initial':column(0),'noise_bits':['3f800000']*6,'limit':32,'ops':[['badlands',2**31-1,-2**31,2**31-1,-64],['frozen',2**31-1,-2**31,2**31-1,-64]]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-material-extension-parity';build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    resources=worldgen.noise_resources(jars['server'])
    if not resources or resources!=worldgen.noise_resources(jars['client']):raise ValueError('reference noise resources differ')
    prefix='data/minecraft/worldgen/noise/';noises={'minecraft:'+name[len(prefix):-5]:raw for name,raw in resources.items()}
    biomes=biome.biome_resources(jars['server'])
    if not biomes or biomes!=biome.biome_resources(jars['client']):raise ValueError('reference biome resources differ')
    for name,data in [('noises',noises),('biomes',biomes)]:(build/(name+'.json')).write_text(json.dumps(data,separators=(',',':')),encoding='utf-8')
    folder=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity';oracles=[folder/(name+'.java') for name in ['VanillaMaterialExtensionOracle','VanillaBiomeTemperatureOracle']]
    sources=[*oracles,Path(__file__),*[lab.ROOT/'tools'/name for name in ['test_biome_temperature_parity.py','test_worldgen_parity.py','test_rust_parity.py','lab.py']],lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/material_extension_fixture.rs']
    tested_sources=base.source_hashes(sources);fixtures=list(corpus(biomes));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'noises.json',build/'biomes.json'];tested_fixtures=base.source_hashes(fixture_paths);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependencies,'-d',classes,*oracles],'compile-java')
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True));binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe');run([binary,'material_extension_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependencies]),'dev.horizonlab.parity.VanillaMaterialExtensionOracle',side,build],side)
        reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt');references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched});print(f'{side}: {matched:,} material-extension output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'extension_calls':sum(len(f['ops']) for f in fixtures),'biome_resources':len(biomes),'noise_resources':len(noises),'constant_noise_fixtures':sum('noise_bits' in f for f in fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual private MaterialSystem erodedBadlandsExtension and frozenOceanExtension methods, with ordered noise-coordinate/block-read/block-write traces and complete resulting columns. Both positional random modes, actual seeded bundled noises, all biome climate settings, same-block identity across state properties, air variants, flowing water, waterlogged nonwater blocks, snow/ice thresholds, repeated calls and height clipping by supplied column providers. Constant overrides cover thresholds, signed zero, NaN/infinity, sea-level integer extremes and bounded provider read failures. Native exception class/message and full chunk/heightmap/postprocessing integration are not compared here. Rust caps extension loops to 1,048,576 iterations; oversized synthetic scans hit the fixture provider limit in both implementations. Full surface-stage and playable parity remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped material-extension parity passed: {report_path}')


if __name__=='__main__':main()
