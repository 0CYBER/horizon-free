#!/usr/bin/env python3
"""Compare seeded material bands and surface queries with pinned vanilla."""
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


def corpus():
    rng=random.Random(0x484f52495a4f4e42414e4453)
    points=[[0,0,0],[1,63,-1],[-17,-64,32],[30000000,319,-30000000],[-2**31,-2**31,2**31-1],[2**31-1,2**31-1,-2**31]]
    points += [[7,y,-11] for y in [-1000,-385,-384,-193,-192,-191,-1,0,1,191,192,193,384]]
    seeds=[0,1,-1,-2**63,2**63-1]+[rng.randrange(-2**63,2**63) for _ in range(27)]
    for i,seed in enumerate(seeds):
        for legacy in [False,True]:
            for child in [False,True]:
                yield {"seed":seed,"legacy":legacy,"child":child,"consume":[-1,0,1,17][i%4],"overrides":[None]*3,"points":points}
    edges=[0,0x80000000,1,0x80000001,0x3dffffff,0x3e000000,0x3e000001,0xbdffffff,0xbe000000,0xbe000001,0x3f800000,0xbf800000,0x4effffff,0x4f000000,0xcf000000,0x7f7fffff,0xff7fffff,0x7f800000,0xff800000,0x7fc00001,0xffc00001]
    edges += [rng.randrange(2**32) for _ in range(128)]
    for i,value in enumerate(edges):
        overrides=[f"{value:x}",f"{edges[(i+7)%len(edges)]:x}",f"{edges[(i+13)%len(edges)]:x}"]
        yield {"seed":seeds[i%len(seeds)],"legacy":bool(i%2),"child":bool(i%3),"consume":i%19,"overrides":overrides,"points":points}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-material-surface-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    resources=worldgen.noise_resources(jars["server"])
    if not resources or resources!=worldgen.noise_resources(jars["client"]):raise ValueError("client/server noise resources differ")
    prefix="data/minecraft/worldgen/noise/"
    noises={"minecraft:"+name[len(prefix):-5]:text for name,text in resources.items()}
    (build/"noises.json").write_text(json.dumps(noises,separators=(",",":")),encoding="utf-8")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMaterialSurfaceOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/material_surface_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs,build/"noises.json"];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"material_surface_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaMaterialSurfaceOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} material-surface output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"surface_queries":sum(len(f["points"]) for f in fixtures),"noise_resources":len(noises),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual MaterialSystem.generateBands and constructor-generated terracotta bands, protected getBand/getSurfaceDepth/getSurfaceSecondary methods. Both random streams, direct and world-root-derived clay_bands child streams, preconsumed inputs, complete 192-state band arrays and subsequent RNG draws. Surface lookups use independently authenticated bundled noise definitions, world-root positional random factories and optional constant-noise overrides for rounding/cast edges, nonfinite float values and signed integer overflow/remainder. Raw float-to-double secondary bits use canonical NaNs. Registry adapter identity binding, full material rule evaluation, eroded badlands/frozen ocean extensions and surface-stage chunk integration remain pending."}
    lab.write_json(report_path,report);print(f"Scoped material-surface parity passed: {report_path}")


if __name__=="__main__":main()
