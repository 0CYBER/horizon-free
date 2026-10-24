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
import zipfile

try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def configured_features(*, include_springs=True):
    leaves=[{'type':name} for name in ['end_island','void_start_platform','end_platform']]
    layers=[{'feature':{'type':'fill_layer','height':height,'state':state},'placement':[]} for height,state in [(32,'water'),(16,{'id':'oak_log','properties':{'axis':'x'}}),(8,'air'),(4064,'stone')]]
    targets=[{'target':{'predicate_type':'block_match','block':'stone'},'state':'iron_ore'},{'target':{'predicate_type':'block_match','block':'oak_log'},'state':'copper_ore'}]
    features=leaves+[{'type':'sequence','features':layers},{'type':'replace_single_block','targets':targets},{'type':'scattered_ore','targets':targets,'size':16,'discard_chance_on_air_exposure':0.5}]
    if include_springs:features.append({'type':'spring_feature','state':{'id':'water','properties':{'falling':'false'}},'requires_block_below':False,'rock_count':5,'hole_count':5,'valid_blocks':['air','stone','dirt']})
    return features


def corpus(facts):
    features=configured_features()+[{'type':'end_podium','active':active} for active in [False,True]]
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
    fixture_index=0
    for legacy in [False,True]:
        for seed in [0,-1]:
            for stage in ['empty','terrain']:
                for fail in [-1,7]:
                    for reject in [0,3]:
                        for placement in chains:
                            for worldgen in [False,True]:
                                yield {'upgrading':fixture_index%3==0,'write_zone':[None,[0,0,0],[1,1,0],[-1,-1,1],[0,0,-1]][fixture_index%5],'probe_states':list(range(fixture_index,len(facts['states']),192)),'worldgen':worldgen,'legacy':legacy,'seed':seed,'stage':stage,'fail':fail,'reject':reject,'placement':placement,'features':features,'tick_commands':ticks,'game_time':9223372036854775807 if worldgen else -9223372036854775808,'tick_order':9223372036854775803 if legacy else -9223372036854775808}
                                fixture_index+=1


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
    def fluid_resources(jar):
        with zipfile.ZipFile(jar) as archive:
            prefix='data/minecraft/tags/fluid/'
            return {'minecraft:'+name[len(prefix):-5]:archive.read(name).decode('utf-8') for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith('.json')}
    fluid_tags=fluid_resources(jars['server'])
    if fluid_tags!=fluid_resources(jars['client']):raise ValueError('reference fluid tags differ')
    (build/'fluid-tags.json').write_text(json.dumps(fluid_tags,separators=(',',':')))
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFeatureRegionOracle.java"
    helper=oracle.with_name("VanillaHeightmapOracle.java")
    placement_helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,placement_helper,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/feature_region_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
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
    edges=[-2147483648,-2147483647,-134217729,-134217728,-17,-16,-1,0,1,15,16,134217727,134217728,2147483646,2147483647]
    zones=[]
    for cx,cz in [(0,0),(-1,1),(-2147483648,-2147483648),(2147483647,2147483647),(134217727,-134217728)]:
        for radius in [-2147483648,-2,-1,0,1,2,7,134217728,2147483647]:
            for x in edges:
                for z in edges:zones.append([cx,cz,radius,x,z])
    rng=random.Random(0x574f524c445a4f4e45)
    for _ in range(10000):zones.append([rng.randrange(-2**31,2**31) for _ in range(5)])
    zone_inputs=build/'zones.json';zone_inputs.write_text(json.dumps(zones,separators=(',',':')))
    heights=[]
    for min_y,height in [(-64,64),(-64,384),(0,256),(2147483647,1),(-2147483648,0),(0,-1),(-2147483648,-1)]:
        for y in [-2147483648,-2147483647,-65,-64,-63,-1,0,1,255,256,319,320,2147483646,2147483647]:heights.append([min_y,height,y])
    for _ in range(10000):heights.append([rng.randrange(-2**31,2**31) for _ in range(3)])
    height_inputs=build/'heights.json';height_inputs.write_text(json.dumps(heights,separators=(',',':')))
    fixture_paths=[height_inputs,zone_inputs,inputs,build/"tags.json",build/"fluid-tags.json",build/"facts-client.json",build/"facts-server.json"]
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"feature_region_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} feature-region output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"direct_state_probes":sum(len(f["probe_states"]) for f in fixtures),"write_zone_cases":len(zones),"upgrade_height_cases":len(heights),"upgrade_lifecycle_cases":54,"block_tag_files":len(tags),"fixtures":len(fixtures),"worldgen_random_modes":[False,True],"resident_chunks_per_fixture":9,"requested_placements":sum(len(f["features"])*2 for f in fixtures),"direct_tick_requests":sum(len(f["tick_commands"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Native FeaturePlacer and three basic block-writing features, a composed sequence of four configured fill layers, single-block replacements, scattered ore and water springs share a world proxy backed by nine actual ProtoChunks. Configured water, horizontal logs, air, out-of-height fills and ore/replacement rules test graph-to-region binding. Initial two-layer stone/dirt terrain, both pre-light heightmap groups, count/square/heightmap/layered modifier chains, repeated placements, direct and WorldgenRandom-wrapped random streams, missing-neighbor errors, rejected writes and injected partial failures are compared. Final complete raw sections, heightmap presence and packed words are compared alongside ordered feature writes, update flags, explicit height queries and subsequent random draws. Rust uses resident registry-backed chunks, lazy heightmap priming and a caller-supplied write callback; imported air/fluid facts and bundled block tags bind independently across authenticated JARs. The proxy supplies storage mutation and actual native default scheduleTick/createTick methods routed through WorldGenTickAccess into each ProtoChunk. Separate block/fluid queues, first-request deduplication, nondefault-state block identity, shared tick-order advancement on duplicates and missing-chunk errors, out-of-height requests and wrapped clock values are compared, including saved queue order and priorities. Active and inactive End podiums additionally exercise native WorldGenRegion.destroyBlock through a subclass transport overriding only block reads and writes; air short-circuit, rejected replacements and write failures retain the generation-time semantics. All block states are also written into resident chunks, queried through feature access, cleared and queried again; contained fluid identity/amount/falling/source/full flags, water-tag membership and every bundled block tag plus an absent tag are compared. Negative coordinates, chunk edges, outside-height and absent chunks are covered. Block/fluid query bindings are an immutable registry-checked shared snapshot. The native horizontal write-zone predicate is compared separately across chunk/block coordinates, negative radii and wrapped subtraction/absolute-value extremes, and gates feature callbacks in selected fixtures. Actual WorldGenRegion.ensureCanWrite also checks the center proto-chunk before/after installing native below-zero retrogen data, with extreme and boundary heights; the generic height interval is independently compared against LevelHeightAccessor. Region feature callbacks and direct accepted/rejected write probes apply the upgrade height before mutation. The Rust access object now reads live center-chunk retrogen metadata before each write. Fifty-four lifecycle cases pair all six pre-light stages with all nine nonempty upgrade targets; one access object observes target-completion cleanup, explicit removal, reinstallation and backward stage changes from accepted write callbacks. Neighbor upgrade state is independent. Actual native highest-generated status and ProtoChunk biome-read availability are compared before and after those sequences, including clamped extreme quart Y. Retrogen activation and the world write effects remain caller-owned. Actual WorldGenRegion construction, full retrogen biome resolution, POIs, block entities, postprocessing execution, lighting, block/fluid tick execution, complete feature registries and complete world generation remain outside this comparison."}

    lab.write_json(report_path,report);print(f"Scoped feature-region parity passed: {report_path}")


if __name__=="__main__":main()
