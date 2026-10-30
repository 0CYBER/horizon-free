#!/usr/bin/env python3
"""Compare fill-layer state binding, column traversal and ordered world mutations with pinned vanilla."""
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


def corpus(facts):
    encode=lambda value:json.dumps(value,separators=(',',':'))
    states=['stone','air','cave_air','void_air','water','oak_log',{'id':'water','properties':{'level':'+01'}},{'id':'snow','properties':{'layers':'٠٢'}},{'id':'oak_log','properties':{'axis':'x'}},{'id':'oak_log','properties':{'axis':'invalid'}},None,False,'missing']
    definitions=[{'type':'fill_layer','height':h,'state':state} for h in [-1,0,1,63,4064,4065,4294967297] for state in states]
    definitions += [{'type':'fill_layer','height':h,'state':'stone'} for h in [None,False,'1',0.9]]
    for legacy in [False,True]:
        for seed in [0,-1]:
            for definition in definitions:
                for fail in [-1,0,1,7,63]:
                    for reject in [0,3]:
                        yield {'definition':encode(definition),'origin':[15,99,15],'world_min':-64,'legacy':legacy,'seed':seed,'reject':reject,'fail':fail,'repeats':2}
    for minimum in [-2147483648,2147483647]:
        for origin in [[-1,0,-1],[2147483647,-2147483648,2147483647],[-2147483648,2147483647,-2147483648]]:
            for height in [0,1,4064]:
                yield {'definition':encode({'type':'fill_layer','height':height,'state':'stone'}),'origin':origin,'world_min':minimum,'legacy':False,'seed':0,'reject':0,'fail':-1,'repeats':2}
    for state in facts['states']:
        yield {'definition':encode({'type':'fill_layer','height':1,'state':state}),'origin':[0,0,0],'world_min':-64,'legacy':False,'seed':0,'reject':0,'fail':-1,'repeats':0}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-fill-layer-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFillLayerOracle.java"
    helper=oracle.with_name("VanillaBlockStateCodecOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/fill_layer_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixture_paths=[];classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    for side,jar in jars.items():
        path=build/f"facts-{side}.json"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaFillLayerOracle","export",path],side+"-export")
        fixture_paths.append(path)
    if json.loads((build/"facts-client.json").read_text())!=json.loads((build/"facts-server.json").read_text()):raise ValueError("reference feature block states differ")
    facts=json.loads((build/"facts-server.json").read_text())
    if len(facts['states'])!=35723 or facts['height_limit']!=4064:raise ValueError("unexpected pinned registry or height limit")
    if sorted(facts['air_ids'])!=sorted(facts['defaults']['minecraft:'+name] for name in ['air','cave_air','void_air']):raise ValueError("unexpected air-state set")
    fixtures=list(corpus(facts));inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths.append(inputs)
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"fill_layer_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaFillLayerOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} fill-layer output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"block_states":len(facts["states"]),"integer_properties":len(facts["integers"]),"requested_placements":sum(f["repeats"] for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual Feature.DIRECT_CODEC decoding of fill-layer features binds every pinned block state through BlockState.CODEC; selected compact/full states exercise integer properties and invalid-property defaults. Native FillLayerFeature.place traces each world-minimum query, ordered X/Z block reads and conditional writes, flags, rejected writes, thrown minimum/read/write failures and accumulated stored states after repeated calls. Air-state identity and height-limit metadata are independently exported from both authenticated JARs and checked. Extreme wrapping coordinates and world minima, both random modes and untouched subsequent random state are compared. Full world effects, region integration of fill-layer features, other configured block features and complete generated worlds remain pending."}


    lab.write_json(report_path,report);print(f"Scoped fill-layer parity passed: {report_path}")


if __name__=="__main__":main()
