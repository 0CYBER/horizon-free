#!/usr/bin/env python3
"""Compare basic feature algorithms and ordered world mutations with pinned vanilla."""
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
    encode=lambda value:json.dumps(value,separators=(',',':'))
    origins=[[cx*16,64,cz*16] for cx in [-1,0,1] for cz in [-1,0,1]]+[[8,64,8],[-32,0,0],[32,0,0],[2147483647,-2147483648,2147483647],[-2147483648,2147483647,-2147483648]]
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef]:
            for kind in ['void_start_platform','end_island','end_platform']:
                for origin in origins:
                    for reject in [0,3]:
                        for fail in [-1,0,7,63]:
                            yield {'definition':encode({'type':kind}),'origin':origin,'legacy':legacy,'seed':seed,'reject':reject,'fail':fail,'repeats':2}
    chains=[[],
        [{'type':'count','count':2}],
        [{'type':'count','count':2},{'type':'in_square'}],
        [{'type':'in_square'},{'type':'count','count':2}],
        [{'type':'count','count':{'type':'uniform','min_inclusive':0,'max_inclusive':3}},
         {'type':'offset','x':{'type':'uniform','min_inclusive':-2,'max_inclusive':2},'y':1,'z':-1}],
        [{'type':'count','count':2},{'type':'height_range','height':{'type':'uniform','min_inclusive':{'above_bottom':0},'max_inclusive':{'below_top':0}}}],
        [{'type':'random_chance','chance':0.5},{'type':'count','count':2}],
        [{'type':'count','count':2},{'type':'random_chance','chance':0.5}],
        [{'type':'cuboid','xz_size':1,'y_size':1},{'type':'random_chance','chance':0.5}],
        [{'type':'fixed_placement','positions':[[0,64,0],[0,64,0],[1,64,1]]}],
        [{'type':'noise_threshold_count','noise_level':0,'below_noise':1,'above_noise':2},{'type':'in_square'}]]
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef]:
            for kind in ['void_start_platform','end_island','end_platform']:
                for chain in chains:
                    for fail in [-1,0,7,63]:
                        for reject in [0,3]:
                            yield {'definition':encode({'type':kind}),'placement':chain,'origin':[0,64,0],
                                   'legacy':legacy,'seed':seed,'reject':reject,'fail':fail,'repeats':2}
    for definition in [None,False,0,[],{}, {'type':'missing'}, {'type':':end_island'}, {'type':'minecraft:end_platform','ignored':0}]:
        yield {'definition':encode(definition),'origin':[0,64,0],'legacy':False,'seed':0,'reject':0,'fail':7,'repeats':1}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-basic-feature-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBasicFeatureOracle.java"
    helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/basic_feature_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    for side,jar in jars.items():
        path=build/f"facts-{side}.json"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBasicFeatureOracle","export",path],side+"-export")
        fixture_paths.append(path)
    if json.loads((build/"facts-client.json").read_text())!=json.loads((build/"facts-server.json").read_text()):raise ValueError("reference feature block states differ")
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"basic_feature_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBasicFeatureOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} basic-feature output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"basic_features":3,"block_states":5,"composed_fixtures":sum("placement" in f for f in fixtures),"requested_placements":sum(f["repeats"] for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual Feature.DIRECT_CODEC and native void-start-platform, End-island and End-platform feature algorithms. A mutable WorldGenLevel proxy logs ordered block reads, writes, update flags, rejected writes, thrown failures and accumulated stored states after repeated placements. Native LevelWriter.setBlockAndUpdate executes through its actual default method. Required default block-state IDs are independently exported and checked across both authenticated JARs. Direct calls and native FeaturePlacer modifier chains share the mutable world and random stream, including reused placer buffers after failures. Both random modes, multiple seeds, nearby/far chunks and extreme wrapped coordinates are compared, with nextLong values after every placement and at fixture end. The End-platform feature invokes its ordinary generation path without block destruction/drops. Actual chunk mutation, neighbor updates, lighting, entity/block drops, full feature registry behavior and complete generated worlds remain outside this comparison."}

    lab.write_json(report_path,report);print(f"Scoped basic-feature parity passed: {report_path}")


if __name__=="__main__":main()
