#!/usr/bin/env python3
"""Compare pre-light chunk status transitions and native ProtoChunk writes."""
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
    defaults=facts['defaults'];air=defaults['minecraft:air']
    stages=['empty','structure_starts','structure_references','biomes','terrain','features']
    choices=[defaults['minecraft:'+name] for name in ['air','cave_air','void_air','stone','water','lava','oak_leaves','oak_slab','seagrass','snow','powder_snow','glass','barrier']]
    for stage in ['empty','terrain']:
        ops=[['stage',stage]]
        for state in range(len(facts['states'])):
            ops.extend([['set',4,5,7,state],['get',4,5,7],['query',4,7],['set',4,5,7,air],['query',4,7]])
        yield {'chunk':[0,0],'min':0,'height':16,'ops':ops+[['dump']]}
    rng=random.Random(0x484f52495a4f4e5354414745)
    for minimum,height in [(-64,384),(0,16),(-16,32),(-2032,4064),(2016,16)]:
        for initial_stage in stages:
            ops=[['stage',initial_stage],['set',0,minimum,0,air],['query',0,0]]
            for index,stage in enumerate(stages+list(reversed(stages))):
                ops.extend([['stage',stage],['query',0,0]])
                for i in range(24):
                    x,z=rng.choice([(0,0),(15,15),(-1,16),(2147483647,-2147483648)])
                    y=minimum+rng.choice([-1,0,height//2,height-1,height])
                    state=rng.choice(choices)
                    ops.extend([['set',x,y,z,state],['get',x,y,z],['query',x,z]])
                if index%3==0:ops.append(['prime',rng.randrange(64)])
            yield {'chunk':[-3,7],'min':minimum,'height':height,'ops':ops+[['dump']]}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-chunk-stage-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaChunkStageOracle.java"
    helper=oracle.with_name("VanillaHeightmapOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/chunk_stage_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity.VanillaChunkStageOracle",mode,path],side+"-"+mode)
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
    run([binary,"chunk_stage_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} chunk-stage output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"operation_counts":dict(Counter(op[0] for fixture in fixtures for op in fixture["ops"])),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual ProtoChunk.setBlockState, direct section reads, heightmap presence/packed data and pre-light persisted-status assignments for empty, structure starts, structure references, biomes, terrain and features. Every pinned block state is inserted/removed under both heightmap groups. Forward/backward status assignments, untouched stale maps, missing-map priming, explicit priming, empty-section AIR writes, cave/void-air collapse, outside-height writes and wrapped X/Z coordinates are compared. Facts are independently exported from both authenticated JARs; block tags are resolved in Rust. This covers stage-dependent mutation and map selection, not the scheduler, persisted metadata, retro-generation, block entities, lighting, whole region behavior or a complete generated world."}

    lab.write_json(report_path,report);print(f"Scoped chunk-stage parity passed: {report_path}")


if __name__=="__main__":main()
