#!/usr/bin/env python3
"""Compare biome temperature sampling, precipitation and cache behavior with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab,test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng=random.Random(0x484f52495a4f4e54454d50)
    bases=[0,0x80000000,1,0x80000001,0x3dcccccc,0x3dcccccd,0x3dccccce,0x3e199999,0x3e19999a,0x3e19999b,0x3e4ccccd,0x3f000000,0x3f800000,0xbf800000,0x7f7fffff,0xff7fffff,0x7f800000,0xff800000,0x7fc00000,0xffc00001]
    bases += [rng.getrandbits(32) for _ in range(28)]
    edges=[-2**31,-33554433,-33554432,-16777217,-16777216,-4097,-4096,-64,-1,0,1,63,64,79,80,81,4095,4096,16777216,16777217,33554431,33554432,2**31-1]
    wrap=lambda value:(value+2**31)%2**32-2**31
    points=[[0,y,0,63] for y in [-64,0,63,79,80,81,82,319,4096]]
    points += [[rng.choice(edges),rng.choice(edges),rng.choice(edges),rng.choice(edges)] for _ in range(96)]
    points += [[rng.randrange(-30000000,30000001),rng.randrange(-64,321),rng.randrange(-30000000,30000001),63] for _ in range(96)]
    # Reuse packed keys while varying the source coordinates and sea level.
    for x,y,z,sea in list(points[:32]):
        points += [[x,y,z,wrap(sea+500)],[wrap(x+2**26),wrap(y+4096),wrap(z-2**26),sea]]
    for base in bases:
        for modifier in ['NONE','FROZEN']:
            for precipitation in [False,True]:
                ops=[['query',*p] for p in points]
                ops += [['fill',1100,-10000,63],['dump'],['query',-8901,1099&255,1099*31,-64],['query',-10000,0,0,63],['dump']]
                yield {'base':f'{base:x}','modifier':modifier,'precipitation':precipitation,'ops':ops}
    for base in [0x3e4ccccd,0x7fc00000,0xffc00001]:
        for modifier in ['NONE','FROZEN']:
            for first in [0,1023]:
                ops=[['fill',1024,0,63],['dump'],['query',first,first&255,first*31,-64],['dump'],['fill',1,5000,63],['dump']]
                child=[['dump'],['query',first,first&255,first*31,-1000],['query',0,4096,0,0],['dump']]
                ops += [['thread',child],['dump'],['query',first,first&255,first*31,-64],['dump']]
                yield {'base':f'{base:x}','modifier':modifier,'precipitation':True,'ops':ops}


def biome_resources(jar):
    prefix='data/minecraft/worldgen/biome/'
    with zipfile.ZipFile(jar) as archive:
        return {'minecraft:'+name[len(prefix):-5]:archive.read(name).decode('utf-8') for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith('.json')}


def codec_corpus(biomes):
    cases=list(biomes.values())+['null','false','0','[]','{}']
    template={'has_precipitation':True,'temperature':0.5,'downfall':0.5,'temperature_modifier':'none'}
    tokens=['null','true','false','0','-0.0','0.1','0.15','0.2','1','-1','1.000000059604644775390625','1.000000059604644775390626','1e40','-1e40','1e1000','-1e1000','1e-1000','"none"','"frozen"','"NONE"','"minecraft:none"','"NaN"','"Infinity"','"0.1"','[]','{}']
    for key in template:
        cases.append(json.dumps({k:v for k,v in template.items() if k!=key}))
        for token in tokens:
            cases.append('{'+','.join(json.dumps(k)+':'+(token if k==key else json.dumps(v)) for k,v in template.items())+'}')
    cases += [json.dumps(dict(template,unknown={'anything':True})),
              '{"has_precipitation":true,"temperature":0.2,"temperature":-0.0,"downfall":0.0}',
              '{"has_precipitation":true,"temperature":0.2,"temperature":null,"downfall":0.0}']
    operations=[['query',0,y,0,63] for y in [-64,0,63,79,80,81,319]]+[['query',12345,100,-9876,63],['query',12345,100,-9876,-1000],['dump']]
    for value in cases:yield {'json':value,'ops':operations}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-biome-temperature-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    biomes=biome_resources(jars['server'])
    if not biomes or biomes!=biome_resources(jars['client']):raise ValueError('client/server biome resources differ')
    (build/'biomes.json').write_text(json.dumps(biomes,separators=(',',':')),encoding='utf-8')
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomeTemperatureOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/biome_temperature_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=[*corpus(),*codec_corpus(biomes)];inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs,build/"biomes.json"];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"biome_temperature_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeTemperatureOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} biome-temperature output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"biome_resources":len(biomes),"codec_inputs":sum("json" in f for f in fixtures),"operations":sum(len(f["ops"]) for f in fixtures),"thread_isolation_fixtures":sum(any(op[0]=="thread" for op in f["ops"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual Biome temperature methods, warm/cold/iceberg-melt predicates and precipitation selection, with both temperature modifiers and arbitrary base float bits. Native private climate/biome constructors provide explicit temperature inputs; unrelated biome fields are null and never queried. Fixed seeded simplex/stack sources, float/double conversion and arithmetic order, height adjustment and sea-level overflow, packed-position aliases, repeated queries with changed sea levels, cache capacity/order/content, NaN recomputation and eviction, and actual native thread-local isolation versus independently owned Rust workers. ClimateSettings map-codec parsing additionally covers all independently authenticated bundled biome definitions, direct float conversion, nonfinite overflow, modifier defaults/null handling, field types, duplicates and missing fields. Full biome data codecs, climate encoding, complete weather placement/freezing, material condition integration and automatic migration of shared biome instances between threads remain outside this scope."}
    lab.write_json(report_path,report);print(f"Scoped biome-temperature parity passed: {report_path}")


if __name__=="__main__":main()
