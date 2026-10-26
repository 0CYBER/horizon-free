#!/usr/bin/env python3
"""Compare placement-modifier decoding and ordered emissions with pinned vanilla."""
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
    encode=lambda value:json.dumps(value,separators=(',',':'))
    for value in [None,False,0,[],{}, {'type':'missing'}, {'type':'in_square'}, {'type':':in_square'}]:yield encode(value)
    for count in [-1,0,1,3,17,4096,4097,4294967297, {'type':'uniform','min_inclusive':0,'max_inclusive':5}, {'type':'clamped','source':10,'min_inclusive':0,'max_inclusive':3}, {'type':'weighted_list','distribution':[{'data':0,'weight':1},{'data':4,'weight':3}]}]:yield encode({'type':'count','count':count})
    for field in ['x','y','z']:
        for value in [-17,-16,-1,0,1,16,17,4294967297, {'type':'uniform','min_inclusive':-16,'max_inclusive':16}, {'type':'trapezoid','min':-7,'max':7,'plateau':0}]:
            definition={'type':'offset','x':0,'y':0,'z':0};definition[field]=value;yield encode(definition)
    for chance in [None,False,'1',-1,-0.,0,1e-100,0.1,0.5,1,2,3,7,16777217,2147483647,4294967297]:
        for kind in ['rarity_filter','random_chance']:yield encode({'type':kind,'chance':chance})
    for height in [{'absolute':0},{'above_bottom':7},{'relative_to_sea_level':-3}, {'type':'uniform','min_inclusive':{'above_bottom':0},'max_inclusive':{'below_top':0}}, {'type':'trapezoid','min_inclusive':{'absolute':-7},'max_inclusive':{'absolute':7}}, {'type':'weighted_list','distribution':[{'data':{'absolute':1},'weight':2},{'data':{'below_top':1},'weight':3}]}]:yield encode({'type':'height_range','height':height})
    fixed=[[0,7,0],[15,20,15],[-1,-64,-1],[-16,99,16],[2147483647,-2147483648,2147483647],[0,7,0]]
    for positions in [[],fixed,[[1,2]],[[1,2,3,4]],[[0,False,0]],[[4294967297,0,-4294967297]]]:yield encode({'type':'fixed_placement','positions':positions})
    for placements in [[],[{'type':'in_square'}],[{'type':'in_square'},{'type':'count','count':3},{'type':'fixed_placement','positions':fixed}],[{'type':'randomly_selected','placements':[{'type':'in_square'},{'type':'count','count':0}]}]]:yield encode({'type':'randomly_selected','placements':placements})
    for kind,field in [('count','count'),('height_range','height'),('offset','y'),('rarity_filter','chance'),('random_chance','chance'),('fixed_placement','positions'),('randomly_selected','placements')]:
        for value in [None,{},[]]:yield encode({'type':kind,field:value})


    for heightmap in ['WORLD_SURFACE_WG','WORLD_SURFACE','OCEAN_FLOOR_WG','OCEAN_FLOOR','MOTION_BLOCKING','MOTION_BLOCKING_NO_LEAVES','world_surface','missing',None,0]:
        yield encode({'type':'heightmap','heightmap':heightmap})
        yield encode({'type':'surface_relative_threshold_filter','heightmap':heightmap})
        for low,high in [(-3,3),(1,0),(-2147483648,2147483647),(2147483647,2147483647),(-2147483648,-2147483648),(None,None)]:
            yield encode({'type':'surface_relative_threshold_filter','heightmap':heightmap,'min_inclusive':low,'max_inclusive':high})
    for depth in [-2147483648,-1,0,1,7,2147483647,4294967297,None,False,'0']:
        yield encode({'type':'surface_water_depth_filter','max_water_depth':depth})

    for ratio in [-2147483648,-9,-1,0,1,5,16,2147483647]:
        for factor in [-1e300,-200,-0.,0,1e-300,1,80,200,1e300]:
            yield encode({'type':'noise_based_count','noise_to_count_ratio':ratio,'noise_factor':factor})
    for offset in [None,-1,0,0.1,1,1e300,'0',False]:
        yield encode({'type':'noise_based_count','noise_to_count_ratio':7,'noise_factor':80,'noise_offset':offset})
    for level in [-1e300,-1,-0.,0,0.1,0.7,1,1e300]:
        for below,above in [(-1,3),(0,0),(1,7),(17,2),(-2147483648,2147483647)]:
            yield encode({'type':'noise_threshold_count','noise_level':level,'below_noise':below,'above_noise':above})

    for xz,y in [(1,1),(2,3),(3,4),(16,16),(0,2),(2,17),({'type':'uniform','min_inclusive':1,'max_inclusive':5},{'type':'uniform','min_inclusive':1,'max_inclusive':3})]:
        for edges,interior in [(True,True),(True,False),(False,True),(False,False),(None,None),(0,True),(True,'false')]:
            yield encode({'type':'cuboid','xz_size':xz,'y_size':y,'include_edges':edges,'include_interior':interior})

    for count in [-1,0,1,3,7,257, {'type':'uniform','min_inclusive':0,'max_inclusive':5}, {'type':'weighted_list','distribution':[{'data':0,'weight':1},{'data':4,'weight':3}]}]:
        yield encode({'type':'count_on_every_layer','count':count})

    truth={'type':'true'};false={'type':'not','predicate':truth};inside={'type':'inside_world_bounds'}
    predicates=[truth,false,inside,{'type':'all_of','predicates':[]},{'type':'any_of','predicates':[]}]
    for offset in [None,[],[0,0],[0,0,0,0],[0,-17,0],[16,16,-16],[0,-16,0],[0,1,0],[-2147483648,-2147483648,-2147483648],[4294967296,4294967297,-4294967297],[0,False,0]]:
        predicates.append({'type':'inside_world_bounds','offset':offset})
    for kind in ['all_of','any_of']:
        for values in [[truth,inside],[false,inside],[inside,truth],[inside,false],[inside,{'type':'not','predicate':inside}],None,False]:
            predicates.append({'type':kind,'predicates':values})
    predicates += [None,False,{},{'type':'not'},{'type':'not','predicate':None},{'type':'missing'}]
    for predicate in predicates:yield encode({'type':'block_predicate_filter','predicate':predicate})
    for direction in ['up','down','north','UP',None]:
        for steps in [0,1,4,32,33]:
            for target in [truth,false,{'type':'not','predicate':{'type':'inside_world_bounds','offset':[0,-16,0]}}]:
                yield encode({'type':'environment_scan','direction_of_search':direction,'target_condition':target,'max_steps':steps})
    for allowed in [None,truth,false,inside,{'type':'inside_world_bounds','offset':[0,16,0]}, {'type':'all_of','predicates':[inside,inside]}]:
        for target in [inside,{'type':'not','predicate':{'type':'inside_world_bounds','offset':[0,-16,0]}}]:
            for direction in ['up','down']:
                yield encode({'type':'environment_scan','direction_of_search':direction,'target_condition':target,'allowed_search_condition':allowed,'max_steps':32})


def corpus():
    positions=[[0,0,0],[-1,63,-1],[-16,-64,16],[2147483647,-2147483648,2147483647]]
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef]:
            for context in [[-64,384,63],[2147483647,-2147483648,2147483647]]:
                for index,definition in enumerate(definitions()):
                    for fail,draw in [(-1,False),(0,False),(2,True)]:
                        # Limit successful 4096-repeat output; a failing consumer still checks the bound.
                        if '4096' in definition and fail==-1:continue
                        value=json.loads(definition)
                        layer=isinstance(value,dict) and value.get('type')=='count_on_every_layer'
                        if layer and context[0]!=-64:continue
                        noise=isinstance(value,dict) and value.get('type') in ['noise_based_count','noise_threshold_count']
                        unsafe=noise and (abs(value.get('noise_to_count_ratio',0))>16 or not isinstance(value.get('noise_offset',0),(int,float)) or abs(value.get('noise_offset',0))>2 or abs(value.get('below_noise',0))>17 or abs(value.get('above_noise',0))>17)
                        for count_only in ([True] if unsafe else [False,True] if noise else [False]):
                            yield {'layer_world':layer,'count_only':count_only,'definition':definition,'context':context,'legacy':legacy,'seed':seed,'positions':positions,'fail':fail,'draw':draw,'height_mode':index%2,'query_fail':1 if index%5==0 and fail==2 else -1,'world_min':max(-2**31,min(2**31-1,context[0]+[-8,0,8][index%3]))}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-placement-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaPlacementOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/placement_fixture.rs",lab.ROOT/"crates/horizon-world/examples/probe/support/placement_queries.rs"]
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
    run([binary,"placement_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaPlacementOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} placement output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"modifier_kinds":17,"predicate_kinds":5,"definitions":len(list(definitions())),"requested_modifications":sum(len(f["positions"]) for f in fixtures if not f["count_only"]),"requested_counts":sum(len(f["positions"]) for f in fixtures if f["count_only"]),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual PlacementModifier.CODEC parsing and modify callbacks for count, in-square, offset, height-range, rarity, random-chance, fixed-position randomly-selected, heightmap, surface-relative-threshold and surface-water-depth modifiers, plus native noise-based and noise-threshold repeating counts and cuboid placement with edge/interior controls and count-on-every-layer scans, predicate filters and environment scans. Logical and inside-world-bounds predicates preserve short-circuiting and offset wrapping; traced world-bound queries expose scan termination, final target tests and failures. Layer scanning uses traced synthetic block strata containing air variants, water/lava, ordinary ground, bubble columns and bedrock; callbacks and injected height/block failures preserve query order and random consumption. Both random modes, repeated calls, zero counts, inclusive provider bounds, nested random choices, anchored and overflowing coordinates, same-chunk fixed filtering, duplicates, malformed fields and failing consumers. Consumer callbacks optionally draw from the shared random stream; subsequent nextLong values verify exact consumption. Contexts use an actual PlacementContext with a traced WorldGenLevel proxy and native noise generator. Heightmap queries use traced synthetic world responses, ordinary/extreme heights and injected query failures; world minimum and generation minimum can differ. Noise count-only calls include extreme ratios, factors, offsets and thresholds without expanding huge position lists. Additional block predicates, biome filtering, configured features, provider encoders, Gaussian providers and complete world generation remain pending."}

    lab.write_json(report_path,report);print(f"Scoped placement parity passed: {report_path}")


if __name__=="__main__":main()
