#!/usr/bin/env python3
"""Compare modifier-chain execution and shared feature random state with pinned vanilla."""
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


def definitions():
    count=lambda value:{'type':'count','count':value}
    square={'type':'in_square'}
    offset={'type':'offset','x':{'type':'uniform','min_inclusive':-16,'max_inclusive':16},'y':1,'z':{'type':'trapezoid','min':-7,'max':7,'plateau':0}}
    height={'type':'height_range','height':{'type':'uniform','min_inclusive':{'above_bottom':8},'max_inclusive':{'below_top':4}}}
    fixed={'type':'fixed_placement','positions':[[0,0,0],[7,64,9],[15,-64,15],[0,0,0],[-1,63,-1],[-16,5,16]]}
    random={'type':'randomly_selected','placements':[count(0),count(3),fixed,square]}
    yield []
    for modifier in [count(0),count(1),count(3),square,offset,height,fixed,random]:yield [modifier]
    for first in [count(0),count(1),count(3),fixed,random]:
        yield [first,square,offset,height]
        yield [first,{'type':'rarity_filter','chance':3},count(2),random]
        yield [first,{'type':'random_chance','chance':0.5},square,fixed]
        yield [first,random,offset,count(3)]
    yield [count(3),count(3),count(3),offset]
    yield [count(2),{'type':'randomly_selected','placements':[random,offset,height]},count(2)]
    for kind in ['WORLD_SURFACE_WG','WORLD_SURFACE','OCEAN_FLOOR_WG','OCEAN_FLOOR','MOTION_BLOCKING','MOTION_BLOCKING_NO_LEAVES']:
        yield [count(3),square,{'type':'heightmap','heightmap':kind},offset]
        yield [fixed,{'type':'surface_relative_threshold_filter','heightmap':kind,'min_inclusive':-3,'max_inclusive':17},count(2)]
    yield [count(3),square,{'type':'surface_water_depth_filter','max_water_depth':7},height]
    for modifier in [
        {'type':'noise_based_count','noise_to_count_ratio':5,'noise_factor':80,'noise_offset':1},
        {'type':'noise_threshold_count','noise_level':0.1,'below_noise':2,'above_noise':7},
        {'type':'noise_threshold_count','noise_level':-0.5,'below_noise':0,'above_noise':3},
    ]:
        yield [modifier,square,offset]
        yield [count(3),square,modifier,height]
    for edges,interior in [(True,True),(True,False),(False,True),(False,False)]:
        cuboid={'type':'cuboid','xz_size':{'type':'uniform','min_inclusive':1,'max_inclusive':3},'y_size':2,'include_edges':edges,'include_interior':interior}
        yield [cuboid,{'type':'random_chance','chance':0.5},offset]
        yield [count(2),square,cuboid]
    for count in [0,1,{'type':'uniform','min_inclusive':0,'max_inclusive':3}]:
        yield [{'type':'count_on_every_layer','count':count},offset]
        yield [{'type':'count','count':2},{'type':'count_on_every_layer','count':count},{'type':'random_chance','chance':0.5}]
    inside={'type':'inside_world_bounds','offset':[0,-16,0]}
    for predicate in [inside,{'type':'not','predicate':inside},{'type':'any_of','predicates':[inside,{'type':'true'}]}]:
        yield [{'type':'count','count':3},height,{'type':'block_predicate_filter','predicate':predicate},square]
        for direction in ['up','down']:
            scan={'type':'environment_scan','direction_of_search':direction,'target_condition':predicate,'max_steps':32}
            yield [fixed,scan,offset]
            yield [{'type':'count','count':2},height,scan,square]
    yield [{'type':'missing'}]


def corpus():
    positions=[[0,0,0],[0,0,0],[-1,63,-1],[2147483647,-2147483648,2147483647]]
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef]:
            for context in [[-64,384,63],[2147483647,-2147483648,2147483647]]:
                for index,modifiers in enumerate(definitions()):
                    layer=any(v.get('type')=='count_on_every_layer' for v in modifiers)
                    if layer and context[0]!=-64:continue
                    for fail,draw,result in [(-1,False,0),(-1,True,1),(-1,True,2),(0,False,1),(2,True,2),(5,True,1)]:
                        yield {'layer_world':layer,'modifiers':modifiers,'context':context,'legacy':legacy,'seed':seed,'positions':positions,'fail':fail,'draw':draw,'result':result,'height_mode':index%2,'query_fail':1 if index%3==0 and fail==2 else -1,'world_min':max(-2**31,min(2**31-1,context[0]+[-8,0,8][index%3]))}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-feature-placement-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFeaturePlacementOracle.java"
    helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/feature_placement_fixture.rs",lab.ROOT/"crates/horizon-world/examples/probe/support/placement_queries.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"feature_placement_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaFeaturePlacementOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} placement output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"modifier_chains":len(list(definitions())),"requested_placements":sum(len(f["positions"]) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual native FeaturePlacer executing decoded placement-modifier chains with a synthetic Feature sink. Empty chains, repeated modifiers, nested random choices, count expansion, filtering, fixed positions, offsets, anchored heights heightmap filters and layered scans are compared. Sink callbacks emit positions, optionally consume the shared random stream, return true/false/mixed results or fail after partial placement. The same native placer is reused across calls, including after errors, exposing retained pending and modified-position buffers. Both random modes, repeated calls, ordinary and overflowing generation contexts and subsequent nextLong values are compared. Predicate filtering and vertical environment scans compare bounds-query order, short-circuiting and failures. Traced heightmap queries include synthetic extreme heights, distinct world/generation minima and injected failures. Configured feature algorithms, block queries, biome checks, registry lifecycle, nested feature placement, Gaussian providers, encoding and complete world generation remain pending. Rust bounds chain length to 1024 and pending/output work to 2^20."}

    lab.write_json(report_path,report);print(f"Scoped placement parity passed: {report_path}")


if __name__=="__main__":main()
