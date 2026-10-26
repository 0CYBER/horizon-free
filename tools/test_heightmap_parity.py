#!/usr/bin/env python3
"""Compare packed heightmaps with the pinned Java chunk and heightmap runtime."""
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
    states, defaults = facts["states"], facts["defaults"]
    air, stone = defaults["minecraft:air"], defaults["minecraft:stone"]
    choices = [defaults["minecraft:"+name] for name in ["air", "cave_air", "void_air", "stone", "water", "lava", "oak_leaves", "oak_slab", "seagrass", "snow", "powder_snow", "glass", "barrier"]]
    rng = random.Random(0x484f52495a4f4e484549474854)
    def fixture(minimum, height, initial, ops, trace=False):
        return {"min": minimum, "height": height, "initial": initial, "ops": ops, "trace": trace}
    # Every state participates in insertion and removal, not just predicate export.
    ops=[]
    for state in range(len(states)):
        ops.extend([["set", 4, 5, 7, state, 63], ["query", 4, 7], ["set", 4, 5, 7, air, 63]])
    yield fixture(0, 16, [], [*ops, ["dump"]])
    # Exercise section boundaries, the full codec height bound and all bit widths.
    for minimum, height in [(-64,384),(0,256),(0,128),(-2032,4064),(2016,16),(-2032,16),(0,16),(0,32),(0,64),(0,512),(0,1024),(0,2032)]:
        initial=[[x,minimum+(x*97+z*13)%height,z,choices[(x+z)%len(choices)]] for x in range(16) for z in range(16)]
        ops=[["prime",63],["dump"]]
        for x in range(16):
            for z in range(16):ops.append(["query",x,z])
        for x,y,z,_ in initial:ops.extend([["set",x,y,z,air,63],["query",x,z]])
        ops.extend([["prime",63],["dump"]])
        yield fixture(minimum,height,initial,ops,height==16)
    # Re-priming empty columns retains old values; invalid lengths take that path.
    for height in [16,32,128,256,384,1024,2032]:
        bits=height.bit_length();words=(256+(64//bits)-1)//(64//bits)
        for fill in ["0","ffffffffffffffff","a55a00ff55ff00aa"]:
            ops=[]
            for t in range(6):ops.append(["raw",t,[fill]*words])
            ops.extend([["prime",0],["dump"],["prime",63],["dump"]])
            for t in range(6):ops.extend([["raw",t,[]],["raw",t,["0"]*(words+1)]])
            for x,z in [(0,0),(15,15),(16,0),(-1,1),(0,16),(-1,0),(2**31-1,2**31-1)]:ops.append(["query",x,z])
            ops.extend([["update",0,2**31-1,0,stone,0],["update",0,-2**31,0,air,0],["dump"]])
            yield fixture(0,height,[[2,height-1,3,stone]],ops,height==16 and fill=="a55a00ff55ff00aa")
    # Mutation and raw edits interleave; repeated columns force downward rescans.
    for scenario in range(64):
        minimum,height=rng.choice([(-64,384),(0,16),(-16,32),(0,128)])
        initial=[]
        for i in range(64):initial.append([rng.randrange(16),minimum+rng.randrange(height),rng.randrange(16),rng.choice(choices)])
        ops=[["prime",63]]
        for i in range(192):
            x,z=rng.choice([(0,0),(15,15),(3,9)])
            y=minimum+rng.choice([0,height-1,rng.randrange(height)])
            state=rng.choice(choices) if i%3 else rng.randrange(len(states))
            ops.extend([["set",x,y,z,state,63],["query",x,z]])
            if i%47==0:ops.extend([["prime",rng.randrange(64)],["dump"]])
        ops.extend([["prime",63],["dump"]])
        yield fixture(minimum,height,initial,ops,scenario<2)
    # All-air sections collapse cave/void air to AIR when read through ProtoChunk.
    for other in [air,stone]:
        initial=[[2,7,3,defaults["minecraft:cave_air"]],[3,7,3,defaults["minecraft:void_air"]],[15,0,15,other]]
        yield fixture(0,16,initial,[["prime",63],["query",2,3],["query",3,3],["dump"]],True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-heightmap-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/heightmap_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity.VanillaHeightmapOracle",mode,path],side+"-"+mode)
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
    run([binary,"heightmap_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} heightmap output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"operation_counts":dict(Counter(op[0] for fixture in fixtures for op in fixture["ops"])),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Six heightmap types, client/persistence flags and predicates for every pinned block state. Air/fluid facts are imported independently from both authenticated JARs and required to agree; block-tag membership is resolved in Rust from bundled resources. State behavioral methods are not ported by that import. Actual vanilla ProtoChunk and Heightmap APIs compare insertion/removal, downward rescans, repeated priming, empty-column retention, raw packed words including padding/unused slots, invalid-length fallback, wrapped column indexing, height queries, errors and selected exact block-read traces. Positive aligned vertical ranges are bounded to the NoiseSettings codec domain; arbitrary height accessors, chunk lifecycle integration, complete terrain/gameplay and rendering remain pending."}
    lab.write_json(report_path,report);print(f"Scoped heightmap parity passed: {report_path}")


if __name__=="__main__":main()
