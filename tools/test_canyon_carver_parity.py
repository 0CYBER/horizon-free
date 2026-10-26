#!/usr/bin/env python3
"""Compare decoded canyon paths, height-dependent widths and random state with vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab,test_rust_parity as base,test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(resources):
    encode=lambda value:json.dumps(value,separators=(',',':'))
    wrap=lambda value:((value+2**31)%2**32)-2**31
    config=next(json.loads(raw) for raw in resources.values() if json.loads(raw)['type']=='minecraft:canyon')
    for context in [[-64,384,63],[0,128,32],[-2032,4064,0],[0,16,8]]:
        for legacy in [False,True]:
            for seed in [0,1,2,3,17,-1,-987654321,0x123456789abcdef]:
                for cx,cz,dx,dz in [(0,0,0,0),(-1,2,1,-1),(107,-59,-4,3),(1875000,-1875000,0,0),(-2**31,2**31-1,0,0),(0,0,8,-8)]:
                    yield {'definition':encode(config),'context':context,'legacy':legacy,'seed':seed,'target':[cx,cz],'origin':[wrap(cx+dx),wrap(cz+dz)],'repeats':2,'fail':-1}
    for mode in range(32):
        shape={**config['shape'],'width_smoothness':[1,2,3,2**31-1][mode%4],
               'vertical_radius_center_factor':[-1,0,0.5,2][mode%4],
               'distance_factor':[-1,0,0.001,0.5,1,2,4,0.75][mode%8],
               'horizontal_radius_factor':[0,0.1,1,3][mode%4],
               'thickness':[-1,0,1,6][mode%4],'y_scale':[-1,0,1,4][mode%4]}
        custom={**config,'shape':shape,'vertical_rotation':[-0.5,0,0.5,1][mode%4],'y':{'absolute':40}}
        yield {'definition':encode(custom),'context':[-64,384,63],'legacy':bool(mode%2),'seed':mode*971,'target':[0,0],'origin':[0,0],'repeats':2,'fail':[-1,0,1,63][mode//8]}
    bad=[None,False,0,[],{}, {'type':'missing'}, {**config,'type':':canyon'}]
    for field in config:
        bad.append({k:v for k,v in config.items() if k!=field})
        for value in [None,False,[],{},'0']:bad.append({**config,field:value})
    for field in config['shape']:
        bad.append({**config,'shape':{k:v for k,v in config['shape'].items() if k!=field}})
        for value in [None,False,[],{},'0']:bad.append({**config,'shape':{**config['shape'],field:value}})
    for value in [-1,-0.0,0,1,2,1e100,-1e100,-1e-100]:bad.append({**config,'probability':value})
    for value in [-1,0,1,2**31-1,4294967296]:bad.append({**config,'shape':{**config['shape'],'width_smoothness':value}})
    for value in bad:
        yield {'definition':encode(value),'context':[-64,384,63],'legacy':False,'seed':0,'target':[0,0],'origin':[0,0],'repeats':1,'fail':63}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-canyon-carver-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCanyonCarverOracle.java"
    helper=oracle.with_name("VanillaAnchorOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/canyon_carver_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    resources=worldgen.registry_resources(jars['server'],'carver')
    if not resources or resources!=worldgen.registry_resources(jars['client'],'carver'):raise ValueError('reference carver resources differ')
    resources_path=build/'carvers.json';resources_path.write_text(json.dumps(resources,separators=(',',':')),encoding='utf-8')
    fixtures=list(corpus(resources));inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs,resources_path];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"canyon_carver_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaCanyonCarverOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} canyon-carver output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"carve_attempts":sum(f["repeats"] for f in fixtures),"bundled_canyon_definitions":1,
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual WorldCarver.DIRECT_CODEC, CanyonWorldCarver.isStartChunk and CanyonWorldCarver.carve using the independently authenticated bundled canyon definition. Compares ordered output cells including duplicates, partial output failures, resulting mask intervals and subsequent random longs across repeated invocations. Covers both outer random modes, local legacy streams, height-dependent width factors, horizontal/vertical radius variation, four generation heights, negative/distant/extreme coordinates and adjacent/far origin chunks. Custom shapes exercise width smoothness, path length, degenerate radii and failure branches; malformed selected codec fields are tested. Gaussian/weighted provider codecs, neighbor-carver seeding/biome selection, blending and complete worlds remain outside this scope. Rust bounds paths to 4096 steps, height to 4064 and total candidates to 2^24 per invocation."}

    lab.write_json(report_path,report);print(f"Scoped canyon-carver parity passed: {report_path}")


if __name__=="__main__":main()
