#!/usr/bin/env python3
"""Compare structure terrain-adaptation samplers with pinned vanilla bytecode."""
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
    rng=random.Random(0x484f52495a4f4e4245415244)
    kinds=["NONE","BURY","BEARD_THIN","BEARD_BOX","ENCAPSULATE"]
    volumes=[[29,29,29,-14,-14,-14,1,1,1,3],[8,9,7,-17,-20,-19,4,5,6,7],
             [3,2,4,100,200,300,1,1,1,5],[1,1,1,0,0,0,1,1,1,1]]
    points=[[0,0,0],[-12,-12,-12],[11,11,11],[12,12,12],[-13,-13,-13],[-2**31,0,2**31-1],[2**31-1,-2**31,0]]
    def fixture(pieces,junctions,affected,pts=None,vol=None):
        return {"pieces":pieces,"junctions":junctions,"affected":affected,"points":points if pts is None else pts,"volumes":volumes if vol is None else vol}
    # The full junction neighborhood spans every position in the 24-cubed kernel.
    yield fixture([],[[0,0,0]],[-100,-100,-100,100,100,100])
    for affected in [None,[-1,-1,-1,1,1,1],[0,0,0,0,0,0]]:
        yield fixture([],[],affected)
        yield fixture([[[0,0,0,7,8,9],"BURY",0]],[[0,0,0]],affected)
    for kind in kinds:
        for delta in [-2**31,-33,-12,-1,0,1,12,33,2**31-1]:
            yield fixture([[[0,0,0,7,8,9],kind,delta]],[],[-100,-100,-100,100,100,100])
    # Bulk floor clipping intentionally differs from scalar containment.
    for affected in [[1,1,1,9,9,9],[-9,-9,-9,-1,-1,-1],[3,5,7,3,5,7]]:
        yield fixture([[[0,0,0,7,8,9],"ENCAPSULATE",0]],[[0,0,0]],affected,vol=[[9,9,9,-16,-16,-16,4,4,4,9]])
    for scenario in range(96):
        pieces=[];junctions=[]
        for i in range(scenario%17):
            low=[rng.randrange(-32,33) for _ in range(3)];high=[v+rng.randrange(17) for v in low]
            pieces.append([low+high,rng.choice(kinds),rng.randrange(-32,33)])
        for i in range(scenario%11):junctions.append([rng.randrange(-32,33) for _ in range(3)])
        affected=None if scenario%13==0 else [-48,-48,-48,48,48,48]
        pts=[[rng.randrange(-60,61) for _ in range(3)] for _ in range(256)]
        vol=[[rng.randrange(1,13),rng.randrange(1,15),rng.randrange(1,13),rng.randrange(-60,0),rng.randrange(-60,0),rng.randrange(-60,0),rng.randrange(1,9),rng.randrange(1,9),rng.randrange(1,9),rng.randrange(8)]]
        f=fixture(pieces,junctions,affected,pts,vol);yield f
        if scenario%8==0:yield fixture(list(reversed(pieces)),list(reversed(junctions)),affected,pts,vol)
    # Wrapped relative arithmetic, volume intersections and extreme ground deltas.
    for coordinate in [-2**31,-2**31+32,2**31-33,2**31-1]:
        low=[coordinate]*3;high=[min(2**31-1,coordinate+8)]*3
        for kind in kinds:
            yield fixture([[low+high,kind,2**31-1]],[[coordinate,coordinate,coordinate]],[-2**31]*3+[2**31-1]*3,
                          [[coordinate]*3,[0,0,0],[-1,-1,-1]],[[3,3,3,coordinate,coordinate,coordinate,16,16,16,2]])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-structure-density-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaStructureDensityOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/structure_density_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    tested_fixtures=base.source_hashes([inputs]);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"structure_density_fixture",inputs,build/"rust.txt"],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaStructureDensityOracle",inputs,reference],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} structure-density output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged([inputs],tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"scalar_queries":sum(len(f["points"]) for f in fixtures),"volumes":sum(len(f["volumes"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual vanilla Beardifier constructor and scalar/bulk samplers from supplied ordered rigid pieces, jigsaw junctions and normalized affected bounds. Five terrain-adjustment modes, the full 24-cubed junction kernel neighborhood, ground-level offsets and extreme wrapping arithmetic, ordered accumulation, nullable/empty affected bounds, clipped strided volumes including scalar/bulk floor-clipping differences and oversized output tails. Floating outputs compared as canonical-NaN raw bits. Live structure discovery/filtering, invalid bounding-box repair, structure placement and terrain integration remain pending. Local Math.pow-derived kernel results do not establish cross-platform math parity."}
    lab.write_json(report_path,report);print(f"Scoped structure-density parity passed: {report_path}")


if __name__=="__main__":main()
