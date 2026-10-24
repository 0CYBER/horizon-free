#!/usr/bin/env python3
"""Compare biome decoration, resident biome zoom, feature writes and heightmaps."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import platform
import random
import subprocess

try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen, test_feature_region_parity as region, test_decoration_parity as decoration
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_feature_region_parity as region
    import test_decoration_parity as decoration


def corpus(facts):
    features=region.configured_features(include_springs=False)
    biomes=[[[0],[1,2],[3,4,5]],[[0],[2],[3,5]],[[1],[2],[4,5]]]
    chains=[
        [{'type':'count','count':2},{'type':'in_square'},{'type':'heightmap','heightmap':'MOTION_BLOCKING'},{'type':'biome'}],
        [{'type':'count_on_every_layer','count':1},{'type':'offset','x':0,'y':1,'z':0},{'type':'biome'}],
        [{'type':'biome'},{'type':'count','count':2},{'type':'heightmap','heightmap':'WORLD_SURFACE'},{'type':'offset','x':1,'y':0,'z':1}],
        [{'type':'count','count':3},{'type':'in_square'},{'type':'heightmap','heightmap':'MOTION_BLOCKING'},{'type':'block_predicate_filter','predicate':{'type':'matching_biomes','biomes':['horizon:b0','horizon:b1']}},{'type':'biome'}],
        [{'type':'biome'},{'type':'block_predicate_filter','predicate':{'type':'volume_match','min':[-1,0,-1],'max':[1,0,1],'match':{'type':'not','predicate':{'type':'matching_biomes','biomes':['horizon:b3']}}}}],
    ]
    for seed in [0,-1]:
        for stage in ['empty','biomes','terrain']:
            for fail in [-1,7]:
                for reject in [0,3]:
                    for placement in chains:
                        for mixed in [False,True]:
                            f=decoration.fixture(biomes,seed,present=[0,1,2,3] if mixed else [0])
                            f.update(features=features,placements=[placement]*len(features),min=-64,height=64,stage=stage,write_fail=fail,reject=reject)
                            yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-decoration-region-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDecorationRegionOracle.java"
    helper=oracle.with_name("VanillaHeightmapOracle.java")
    placement_helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,placement_helper,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_feature_region_parity.py",lab.ROOT/"tools/test_decoration_parity.py",lab.ROOT/"tools/test_feature_sort_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/decoration_region_fixture.rs",lab.ROOT/"crates/horizon-world/examples/probe/support/decoration_storage.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper,placement_helper],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity.VanillaDecorationRegionOracle",mode,path],side+"-"+mode)
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
    run([binary,"decoration_region_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} decoration-region output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"biome_zoom":True,"resident_chunks_per_fixture":9,"requested_decorations":len(fixtures)*2,
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual ChunkGenerator.applyBiomeDecoration with structures disabled executes three native basic features, configured fill-layer sequences, replacements and scattered ore into nine ProtoChunks. Biome palettes, native BiomeManager zoom, possible-biome selection, FeatureSorter order, WorldgenRandom seeding, biome filters, registry-bound matching-biome predicates, ordered volume predicates and graph composition share resident terrain. Both pre-light heightmap groups, mixed/uniform biome neighborhoods, count/square/heightmap/layered chains, repeated decoration, rejected writes and partial failures are compared. Ordered writes, current-feature markers, complete final raw sections and all heightmap presence/data are checked. Rust uses resident BiomeRegion and TerrainRegion with caller-owned biome membership/write effects. Actual WorldGenRegion write-zone policy, POIs, block entities, postprocessing, lighting, structures, remaining feature algorithms, complete noise-to-decoration lifecycle and gameplay remain pending."}


    lab.write_json(report_path,report);print(f"Scoped decoration-region parity passed: {report_path}")


if __name__=="__main__":main()
