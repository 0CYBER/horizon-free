#!/usr/bin/env python3
"""Compare fixed, checkerboard and End biome sources with pinned vanilla."""
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
    rng=random.Random(0x484f52495a4f4e534f55524345)
    edges=[-2**31,-2**31+1,-536870913,-536870912,-260,-257,-256,-255,-4,-1,0,1,4,255,256,257,260,536870911,536870912,2**31-1]
    positions=[[x,y,z] for x in edges for z in edges for y in [-2**31,-1,0,79,2**31-1]]
    positions += [[rng.randrange(-2**31,2**31) for _ in range(3)] for _ in range(512)]
    yield {'kind':'fixed','positions':positions}
    for scale in [-2**31,-65,-34,-33,-32,-3,-2,-1,*range(63),63,64,65,2**31-1]:
        for biomes in [[],[0],[3,1,3,2],list(range(67)),list(range(512))]:yield {'kind':'checkerboard','scale':scale,'biomes':biomes,'positions':positions}
    bits=['00000000','80000000','3e800000','3e7fffff','3e800001','bd800000','bd800001','bd7fffff','be600000','be600001','be5fffff','7f800000','ff800000','7fc00000','7f800001','ffc00000']
    bits += [f'{rng.getrandbits(32):08x}' for _ in range(64)]
    for value in bits:yield {'kind':'end','bits':value,'fail':False,'positions':positions}
    yield {'kind':'end','bits':'00000000','fail':True,'positions':positions}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-biome-source-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomeSourceOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/biome_source_fixture.rs"]
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
    run([binary,"biome_source_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeSourceOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} biome-source output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"queries":sum(len(f["positions"]) for f in fixtures),"sources":{name:sum(f["kind"]==name for f in fixtures) for name in ["fixed","checkerboard","end"]},
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual FixedBiomeSource, CheckerboardColumnBiomeSource and TheEndBiomeSource runtime queries using synthetic holder identities. Fixed results, checkerboard empty/duplicate/large lists, all codec-valid scales plus unrestricted runtime scales, Java masked shifts and wrapping sums are compared. End central-circle shortcuts, erosion sampler coordinates, exact threshold boundaries, nonfinite float samples, sampler failures and integer coordinate wrapping are compared through actual Climate.Sampler bindings. End density inputs in this suite are supplied constants; real seeded End density, biome-source codecs, registry/preset loading, possible-biome metadata and generated-world integration remain separate work."}

    lab.write_json(report_path,report);print(f"Scoped biome-source parity passed: {report_path}")


if __name__=="__main__":main()
