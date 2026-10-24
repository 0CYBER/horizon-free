#!/usr/bin/env python3
"""Compare resident bulk-section caching, raw mutations and unchanged heightmaps."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import platform
import random
import subprocess

try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(facts):
    ids=facts['defaults'];count=len(facts['states']);rng=random.Random(6541)
    for minimum in [-2032,-64,0,1968]:
        for prime in [False,True]:
            ops=[]
            for x,y,z in [[0,minimum,0],[0,minimum+16,0],[16,minimum+31,-1],[-1,minimum+63,16],[0,minimum-1,0],[2147483647,2147483647,-2147483648],[1600,minimum,1600]]:
                ops += [['has',x,y,z],['get',x,y,z],['set',x,y,z,ids['minecraft:cave_air']],['get',x,y,z],['world',x,y,z]]
            selected=range(count) if minimum==-64 else [rng.randrange(count) for _ in range(64)]
            for state in selected:
                ops += [['set',0,minimum+1,0,state],['get',0,minimum+1,0],['world',0,minimum+1,0]]
            for state in [ids['minecraft:air'],ids['minecraft:stone'],ids['minecraft:void_air'],ids['minecraft:cave_air']]:
                ops += [['set',1,minimum+48,1,state],['get',1,minimum+48,1],['world',1,minimum+48,1],['height','WORLD_SURFACE',1,1]]
            # These X positions alias in the native 22-bit packed section key.
            for x in [0,67108864,0]:
                ops += [['set',x,minimum+2,0,ids['minecraft:stone']],['get',x,minimum+2,0],['world',x,minimum+2,0]]
            ops += [['reset']]
            for x in [67108864,0,67108864]:
                ops += [['set',x,minimum+2,0,ids['minecraft:water']],['get',x,minimum+2,0],['world',x,minimum+2,0]]
            yield {'min':minimum,'height':64,'prime':prime,'chunks':[[0,0],[1,-1],[-1,1],[4194304,0]],'ops':ops}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-bulk-section-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBulkSectionOracle.java"
    helper=oracle.with_name("VanillaHeightmapOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/bulk_section_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity.VanillaBulkSectionOracle",mode,path],side+"-"+mode)
    for side in jars:java(side,"export",build/f"facts-{side}.json")
    facts=json.loads((build/"facts-server.json").read_text())
    if facts!=json.loads((build/"facts-client.json").read_text()):raise ValueError("client/server state facts differ")
    if len(facts["states"])!=35723:raise ValueError("incomplete pinned state table")
    fixtures=list(corpus(facts));inputs=build/"inputs.txt"
    inputs.write_text("".join(json.dumps(value,separators=(",",":"))+"\n" for value in fixtures),encoding="utf-8")
    (build/"tags.json").write_text(json.dumps(tags,separators=(",",":")),encoding="utf-8")
    fixture_paths=[inputs,build/"tags.json",build/"facts-client.json",build/"facts-server.json"]
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"bulk_section_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} bulk-section output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"resident_chunks_per_fixture":4,"operations":dict(Counter(op[0] for f in fixtures for op in f["ops"])),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual BulkSectionAccess over resident ProtoChunks: raw reads/writes, all-air chunk-read collapse, outside-height AIR, absent sections, missing neighbors, packed-key caching and aliasing, cache reset and complete final raw sections/heightmap words. Every pinned block state is inserted and read with both preprimed and lazy heightmaps. Rust holds an exclusive region borrow instead of Java section acquire/release locks. Concurrent ownership, ordinary ore geometry, region side effects and complete gameplay remain pending."}

    lab.write_json(report_path,report);print(f"Scoped bulk-section parity passed: {report_path}")


if __name__=="__main__":main()
