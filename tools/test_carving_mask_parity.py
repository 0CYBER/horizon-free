#!/usr/bin/env python3
"""Compare packed carving-mask runs, callback mutations and failure state with vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab,test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng=random.Random(0x484f52495a4f4e4341525645)
    carve=lambda x,y,z:{'kind':'carve','pos':[x,y,z]}
    fill=lambda x0,x1,y0,y1,z0,z1:{'kind':'fill','box':[x0,x1,y0,y1,z0,z1]}
    visit=lambda fail=-1,add=None:{'kind':'visit','fail':fail,'add':add or {}}
    wrap=lambda value:((value+2**31)%2**32)-2**31
    for minimum,height in [(0,1),(-1,2),(-64,3),(-64,63),(0,64),(10,65),(-64,384),(-2032,4064),(-2**31,65),(2**31-65,65)]:
        maximum=minimum+height-1
        # Isolated cells, duplicate insertions, packed word/column boundaries,
        # Java-wrapped coordinate aliases, negative indices and positive growth.
        ops=[visit(),{'kind':'read'}]
        points=[[0,minimum,0],[15,maximum,15],[0,maximum,0],[0,minimum,1],[1,minimum,0]]
        points += [[rng.randrange(16),minimum+rng.randrange(height),rng.randrange(16)] for _ in range(512)]
        for x in [-2**31,-2**31+1,-268435456,-1,0,15,16,2**31-1]:
            for y in [wrap(minimum-1),minimum,maximum,wrap(maximum+1)]:
                for z in [-17,-1,0,16,17]:
                    index=wrap(wrap(y-minimum)+wrap(wrap(z+wrap(x<<4))*height))
                    if index<0 or index<2**20:points.append([x,y,z])
        for i,(x,y,z) in enumerate(points):
            ops += [carve(x,y,z)]
            if i%97==0:ops += [visit()]
        ops += [visit(0),visit(63),visit(),{'kind':'read'}]
        yield {'min':minimum,'max':maximum,'ops':ops}
        # Runs spanning all columns and word boundaries, duplicate dense writes.
        yield {'min':minimum,'max':maximum,'ops':[fill(0,15,minimum,maximum,0,15),visit(),fill(0,1,minimum,maximum,0,1),{'kind':'read'}]}
        if height>=3:
            # Mutation behind/ahead of the cursor, extending its current run,
            # joining runs, and a failing visitor retaining its added bits.
            yield {'min':minimum,'max':maximum,'ops':[
                carve(0,minimum,1),carve(0,minimum+2,1),carve(2,minimum,0),
                visit(-1,{'0':[[0,minimum,0],[0,minimum+1,1],[0,minimum+2,1],[1,maximum,0]],'1':[[3,maximum,15]],'2':[[4,minimum,1]]}),
                visit(0,{'0':[[5,maximum,1]]}),{'kind':'read'},visit()]}



def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-carving-mask-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCarvingMaskOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/carving_mask_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"carving_mask_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaCarvingMaskOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} carving-mask output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"operations":sum(len(f["ops"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual CarvingMask construction, carve, isEmpty and visit methods. Compares packed vertical-run traversal, splitting across columns, dense/sparse masks, duplicate writes, word boundaries, coordinate overflow and positive aliases, negative index failures, callback additions during traversal and mutations retained after visitor failures. Rust bounds height to 1..4064 and positive index growth to 2^24 bits; corpus indices remain below 2^20. This validates masks, not cave/canyon generation, mask application, aquifers, blending or complete terrain generation."}

    lab.write_json(report_path,report);print(f"Scoped carving-mask parity passed: {report_path}")


if __name__=="__main__":main()
