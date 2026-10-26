#!/usr/bin/env python3
"""Compare inline feature graphs and ordered world mutations with pinned vanilla."""
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
    placed=lambda kind,mods=[]:{'feature':{'type':kind},'placement':mods}
    noop=placed('no_op');false=placed('no_op',[{'type':'count','count':0}]);island=placed('end_island')
    platform=placed('end_platform',[{'type':'offset','x':1,'y':2,'z':-1}])
    yield {'type':'no_op'}
    for kind in ['sequence','overlay','simple_random_selector']:
        for features in [[],[noop],[false],[island],[false,island],[island,false],[false,false],[noop,false,island,platform],[noop,island,platform],None,noop]:
            yield {'type':kind,'features':features}
    for yes,no in [(noop,false),(false,island),(island,platform),(false,false)]:
        yield {'type':'random_boolean_selector','feature_true':yes,'feature_false':no}
    for chance in [-1,-0.,0,1e-100,0.25,0.5,1,1.01,None,False,'0.5']:
        for selected in [false,island]:
            yield {'type':'random_selector','features':[{'feature':selected,'chance':chance},{'feature':platform,'chance':0.75}],'default':island}
    yield {'type':'random_selector','features':[],'default':noop}
    inner={'type':'random_boolean_selector','feature_true':island,'feature_false':false}
    nested={'feature':inner,'placement':[{'type':'count','count':2},{'type':'in_square'}]}
    yield {'type':'sequence','features':[noop,nested,platform]}
    yield {'type':'simple_random_selector','features':[nested,platform,false]}
    overlay={'feature':{'type':'overlay','features':[false,nested,false,platform]},'placement':[{'type':'count','count':2},{'type':'in_square'}]}
    yield {'type':'overlay','features':[false,overlay,nested,platform]}
    yield {'type':'sequence','features':[overlay,nested]}
    yield {'type':'random_selector','features':[{'feature':nested,'chance':0},{'feature':false,'chance':1}],'default':island}
    for malformed in [None,False,{},'missing',{'feature':{'type':'no_op'}},{'feature':{'type':'no_op'},'placement':None}]:
        yield {'type':'sequence','features':[malformed]}
        yield {'type':'overlay','features':[noop,malformed]}
        yield {'type':'random_boolean_selector','feature_true':noop,'feature_false':malformed}

    for weights in [[],[0],[1],[63],[64],[65],[2147483647],[0,0],[0,1],[1,0],[1,3],[31,32],[31,33],[31,34],[2147483647,1],[-1,2],[0.9,1],[False,1],['1',2],[4294967297,1]]:
        yield {'type':'weighted_random_selector','features':[{'data':[noop,false,island,platform][i%4],'weight':w} for i,w in enumerate(weights)]}
    for data in [noop,false,island,platform,nested,{'feature':{'type':'missing'},'placement':[]}]:
        for weights in [(0,0),(0,1),(1,0),(1,3)]:
            yield {'type':'weighted_random_selector','features':[{'data':data,'weight':weights[0]},{'data':island,'weight':weights[1]}]}


def corpus():
    encode=lambda value:json.dumps(value,separators=(',',':'))
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef]:
            for definition in definitions():
                for fail in [-1,0,7]:
                    for reject in [0,3]:
                        yield {'definition':encode(definition),'origin':[15,64,15],'legacy':legacy,'seed':seed,'reject':reject,'fail':fail,'repeats':2}
    for definition in definitions():
        yield {'definition':encode(definition),'placement':[{'type':'count','count':2},{'type':'in_square'}],'origin':[0,64,0],'legacy':False,'seed':17,'reject':0,'fail':-1,'repeats':2}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-feature-graph-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFeatureGraphOracle.java"
    helper=oracle.with_name("VanillaPlacementOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/feature_graph_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    for side,jar in jars.items():
        path=build/f"facts-{side}.json"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaFeatureGraphOracle","export",path],side+"-export")
        fixture_paths.append(path)
    if json.loads((build/"facts-client.json").read_text())!=json.loads((build/"facts-server.json").read_text()):raise ValueError("reference feature block states differ")
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"feature_graph_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaFeatureGraphOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} feature-graph output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"basic_features":3,"composition_features":7,"definitions":len(list(definitions())),"block_states":5,"composed_fixtures":sum("placement" in f for f in fixtures),"requested_placements":sum(f["repeats"] for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual Feature.DIRECT_CODEC and native no-op, sequence, overlay, random-boolean-selector, simple-random-selector and chance-based random-selector and weighted-random-selector features compose three native block-writing features through inline PlacedFeature holders and actual nested FeaturePlacer calls. Ordered writes, reads, flags, accumulated states, boolean results, rejected writes, partial failures, shared randomness and top-level modifier chains are compared. Overlay exhaustive ordered execution with accumulated success, sequence short-circuiting, selector return propagation and empty/all-zero weighted choices without random draws are included. Named registry holders/tags, encoders, full feature registries and complete generated worlds remain pending. Rust bounds decode depth/nodes/lists, nested execution depth and feature/placed-feature visits; nested modifier-chain execution shares the feature work budget."}


    lab.write_json(report_path,report);print(f"Scoped feature-graph parity passed: {report_path}")


if __name__=="__main__":main()
