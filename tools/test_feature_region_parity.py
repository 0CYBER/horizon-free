#!/usr/bin/env python3
"""Compare feature chains, shared resident chunks and live heightmap mutations."""
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
    leaves=[{'type':name} for name in ['end_island','void_start_platform','end_platform']]
    layers=[{'feature':{'type':'fill_layer','height':height,'state':state},'placement':[]} for height,state in [(32,'water'),(16,{'id':'oak_log','properties':{'axis':'x'}}),(8,'air'),(4064,'stone')]]
    targets=[{'target':{'predicate_type':'block_match','block':'stone'},'state':'iron_ore'},{'target':{'predicate_type':'block_match','block':'oak_log'},'state':'copper_ore'}]
    features=leaves+[{'type':'sequence','features':layers},{'type':'replace_single_block','targets':targets},{'type':'scattered_ore','targets':targets,'size':16,'discard_chance_on_air_exposure':0.5}]
    features.append({'type':'spring_feature','state':{'id':'water','properties':{'falling':'false'}},'requires_block_below':False,'rock_count':5,'hole_count':5,'valid_blocks':['air','stone','dirt']})
    get=lambda name:facts['defaults']['minecraft:'+name]
    log_x=next(i for i,state in enumerate(facts['states']) if state[4]['id']=='minecraft:oak_log' and state[4].get('properties',{}).get('axis')=='x')
    ticks=[]
    for p in [[0,-40,0],[-1,-64,-1],[16,100,16],[32,0,0],[-17,0,0],[2147483647,-2147483648,0]]:
        for kind,value in [('block',get('water')),('fluid',2),('block',log_x),('block',get('oak_log'))]:
            for delay in [-2147483648,0,2147483647]:ticks.append([kind,value,p,delay])
    for kind in range(5):ticks.append(['fluid',kind,[0,-40,0],-1])
    chains=[
        [{'type':'count','count':2},{'type':'in_square'},{'type':'heightmap','heightmap':'MOTION_BLOCKING'}],
        [{'type':'count_on_every_layer','count':1},{'type':'offset','x':0,'y':1,'z':0}],
        [{'type':'count','count':2},{'type':'heightmap','heightmap':'WORLD_SURFACE'},{'type':'offset','x':1,'y':0,'z':1}],
    ]
    for legacy in [False,True]:
        for seed in [0,-1]:
            for stage in ['empty','terrain']:
                for fail in [-1,7]:
                    for reject in [0,3]:
                        for placement in chains:
                            for worldgen in [False,True]:
                                yield {'worldgen':worldgen,'legacy':legacy,'seed':seed,'stage':stage,'fail':fail,'reject':reject,'placement':placement,'features':features,'tick_commands':ticks,'game_time':9223372036854775807 if worldgen else -9223372036854775808,'tick_order':9223372036854775803 if legacy else -9223372036854775808}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-feature-region-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFeatureRegionOracle.java"
    helper=oracle.with_name("VanillaHeightmapOracle.java")
    placement_helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,placement_helper,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/feature_region_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper,placement_helper],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity.VanillaFeatureRegionOracle",mode,path],side+"-"+mode)
    for side in jars:java(side,"export",build/f"facts-{side}.json")
    facts=json.loads((build/"facts-server.json").read_text())
    if facts!=json.loads((build/"facts-client.json").read_text()):raise ValueError("client/server state facts differ")
    if len(facts["states"])!=35723:raise ValueError("incomplete pinned state table")
    fixtures=list(corpus(facts));inputs=build/"inputs.txt"
    inputs.write_text("".join(json.dumps(value,separators=(",",":"))+"\n" for value in fixtures),encoding="utf-8")
    (build/"tags.json").write_text(json.dumps(tags,separators=(",",":")),encoding="utf-8")
    fixture_paths=[inputs,build/"tags.json",build/"facts-client.json",build/"facts-server.json"]
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","feature_region_fixture"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("feature_region_fixture.exe" if os.name=="nt" else "feature_region_fixture")
    run([binary,build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} feature-region output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"worldgen_random_modes":[False,True],"resident_chunks_per_fixture":9,"requested_placements":sum(len(f["features"])*2 for f in fixtures),"direct_tick_requests":sum(len(f["tick_commands"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Native FeaturePlacer and three basic block-writing features, a composed sequence of four configured fill layers, single-block replacements, scattered ore and water springs share a world proxy backed by nine actual ProtoChunks. Configured water, horizontal logs, air, out-of-height fills and ore/replacement rules test graph-to-region binding. Initial two-layer stone/dirt terrain, both pre-light heightmap groups, count/square/heightmap/layered modifier chains, repeated placements, direct and WorldgenRandom-wrapped random streams, missing-neighbor errors, rejected writes and injected partial failures are compared. Final complete raw sections, heightmap presence and packed words are compared alongside ordered feature writes, update flags, explicit height queries and subsequent random draws. Rust uses resident registry-backed chunks, lazy heightmap priming and a caller-supplied write callback; imported air/fluid facts and bundled block tags bind independently across authenticated JARs. The proxy supplies storage mutation and actual native default scheduleTick/createTick methods routed through WorldGenTickAccess into each ProtoChunk. Separate block/fluid queues, first-request deduplication, nondefault-state block identity, shared tick-order advancement on duplicates and missing-chunk errors, out-of-height requests and wrapped clock values are compared, including saved queue order and priorities. Actual WorldGenRegion construction, write-zone policy, POIs, block entities, postprocessing execution, lighting, block/fluid tick execution, complete feature registries and complete world generation remain outside this comparison."}

    lab.write_json(report_path,report);print(f"Scoped feature-region parity passed: {report_path}")


if __name__=="__main__":main()
