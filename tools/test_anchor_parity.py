#!/usr/bin/env python3
"""Compare vertical anchor codecs, resolution and generation bounds with pinned vanilla."""
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
    rng=random.Random(0x484f52495a4f4e414e43484f52)
    names=["absolute","above_bottom","below_top","relative_to_sea_level"]
    edges=[-2**31,-2033,-2032,-64,-1,0,1,63,2031,2032,2**31-1]
    values=edges+[rng.randrange(-2**31,2**31) for _ in range(128)]
    encodings=['null','false','0','"absolute"','[]','{}']
    numbers=[str(v) for v in edges]+['0.9','-0.9','1e3','-1e3','4294967296','4294967295','-4294967296','1e100','-1e100','1e-100','2147483648.9','-2147483649.9','null','true','"12"','[]','{}']
    for name in names:
        for value in numbers:encodings.append('{"'+name+'":'+value+'}')
        encodings += ['{"'+name+'":0,"unknown":true}','{"'+name+'":0,"'+name+'":1}','{"'+name+'":1,"'+name+'":null}']
    import itertools
    for combination in itertools.product([None,0,2032,"bad"],repeat=4):
        encodings.append(json.dumps({name:value for name,value in zip(names,combination) if value is not None},separators=(",",":")))
    contexts=[[-64,384,63],[0,256,32],[-2032,4064,0],[100,0,-100],[0,-1,0]]
    contexts += [[x,y,z] for x,y,z in itertools.product([-2**31,0,2**31-1],repeat=3)]
    contexts += [[rng.randrange(-2**31,2**31) for _ in range(3)] for _ in range(64)]
    for i,context in enumerate(contexts):
        for generator in [None,contexts[(i+7)%len(contexts)]]:
            fixture={"context":context,"values":values,"json":encodings}
            if generator is not None:fixture["generator"]=generator
            yield fixture


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-anchor-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaAnchorOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/anchor_fixture.rs"]
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
    run([binary,"anchor_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaAnchorOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} anchor output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"codec_inputs":sum(len(f["json"]) for f in fixtures),"runtime_anchors":sum(3+4*len(f["values"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual VerticalAnchor.CODEC parsing and encoding, all four anchor kinds, constant helpers, runtime integer overflow and unrestricted constructor values. WorldGenerationContext.of uses a LevelAccessor proxy with verified method calls; the generator constructor uses a native NoiseBasedChunkGenerator with supplied minimum, height and sea level. Strict JSON transport covers range limits, numeric truncation/wrapping, null/wrong field types, duplicate fields and nested exclusive alternatives. Level access outside the three queried values, JSON syntax diagnostics and complete material conditions remain pending."}
    lab.write_json(report_path,report);print(f"Scoped anchor parity passed: {report_path}")


if __name__=="__main__":main()
