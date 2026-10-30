#!/usr/bin/env python3
"""Compare shared cave/canyon geometry and path reachability with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import struct
import math
try:
    from . import lab,test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def bits(value):
    return struct.pack('>d',value).hex()


def corpus():
    rng=random.Random(0x484f52495a4f4e454c4c4950)
    wrap=lambda value:((value+2**31)%2**32)-2**31
    for cx,cz in [(0,0),(-1,2),(1875000,-1875000),(-2**31,2**31-1),(134217727,-134217728)]:
        bx,bz=wrap(cx<<4),wrap(cz<<4)
        for minimum,height in [(-64,384),(0,128),(-2032,4064)]:
            ellipsoids=[]
            for index in range(16):
                center=[bx+rng.uniform(-8,24),rng.uniform(minimum-8,minimum+height+8),bz+rng.uniform(-8,24)]
                horizontal=[0.1,1,4,8,16,32,64,0][index%8]
                vertical=[0.1,0.5,1,4,32,64,512,0][index%8]
                ellipsoids.append({'center':list(map(bits,center)),'horizontal':bits(horizontal),'vertical':bits(vertical),'skip':index%4,'fail':[-1,0,1,63][index//4]})
            # Exact boundary and IEEE inputs, bounded by real output height.
            for index,value in enumerate([0.,-0.,-1.,math.inf,-math.inf,math.nan,2**31,-2**31,1e300,5e-324]):
                ellipsoids += [
                    {'center':list(map(bits,[bx+8,minimum+height/2,bz+8])),'horizontal':bits(value),'vertical':bits(4),'skip':index%4,'fail':63},
                    {'center':list(map(bits,[bx+8,minimum+height/2,bz+8])),'horizontal':bits(4),'vertical':bits(value),'skip':index%4,'fail':63},
                    {'center':list(map(bits,[value,minimum+height/2,bz+8])),'horizontal':bits(4),'vertical':bits(4),'skip':index%4,'fail':63}]
            yield {'chunk':[cx,cz],'min':minimum,'max':minimum+height-1,'ellipsoids':ellipsoids}


def reach_corpus():
    rng=random.Random(0x434152565245414348)
    wrap=lambda value:((value+2**31)%2**32)-2**31
    fbits=lambda value:struct.pack('>f',value).hex()
    for cx,cz in [(0,0),(-1,2),(1875000,-1875000),(-2**31,2**31-1)]:
        bx,bz=wrap(cx<<4)+8,wrap(cz<<4)+8
        for thickness in [0,1,-18,-20,math.inf,-math.inf,math.nan,1e30,0.00001]:
            for dx in [0,1,18,19,100,math.inf,-math.inf,math.nan]:
                for start,end in [(0,0),(0,112),(111,112),(-2**31,2**31-1),(2**31-1,-2**31)]:
                    yield [cx,cz,bits(bx+dx),bits(bz+dx),start,end,fbits(thickness)]
        for _ in range(1024):
            yield [cx,cz,bits(bx+rng.uniform(-300,300)),bits(bz+rng.uniform(-300,300)),rng.randrange(-100,100),rng.randrange(-100,100),fbits(rng.uniform(-20,50))]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-carving-geometry-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCarvingGeometryOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/carving_geometry_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    reach=list(reach_corpus());reach_path=build/'reach.txt';reach_path.write_text(''.join(json.dumps(q,separators=(',',':'))+'\n' for q in reach),encoding='utf-8')
    fixture_paths=[inputs,reach_path];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"carving_geometry_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaCarvingGeometryOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} carving-geometry output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"ellipsoids":sum(len(f["ellipsoids"]) for f in fixtures),"reach_queries":len(reach),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual WorldCarver.carveEllipsoid and canReach methods. Compares ordered normalized-coordinate skip queries, output cells, partial output on callback failure and resulting mask column runs; canReach includes Java float thickness arithmetic and signed step overflow. Corpus covers negative/extreme/distant chunk coordinates, output-height clipping, small/large/zero/negative radii, boundaries and IEEE NaNs/infinities/subnormals. NaN trace values are canonicalized. This validates shared geometric primitives, not randomized cave/canyon paths, codecs, provider sampling, biome carver selection or complete worlds."}

    lab.write_json(report_path,report);print(f"Scoped carving-geometry parity passed: {report_path}")


if __name__=="__main__":main()
