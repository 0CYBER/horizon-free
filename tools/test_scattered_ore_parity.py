#!/usr/bin/env python3
"""Compare scattered-ore attempts, air exposure and rule-driven mutations."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab,test_rust_parity as base,test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(facts):
    yes={'predicate_type':'always_true'}
    no={'predicate_type':'not','rule':yes}
    targets=[[],[{'target':yes,'state':'gold_ore'}],[{'target':no,'state':'air'},{'target':{'predicate_type':'random_block_match','block':'stone','probability':0.5},'state':'iron_ore'},{'target':yes,'state':'copper_ore'}]]
    def fixture(definition,legacy=False,seed=0,mode=0,fail=-1,reject=0,origin=None):
        return {'definition':json.dumps(definition,separators=(',',':')),'origin':origin or [15,-64,15],'world_min':-64,'legacy':legacy,'seed':seed,'mode':mode,'reject':reject,'fail':fail,'repeats':2}
    for size in [0,1,7,8,64]:
        for chance in [0,0.25,0.5,1]:
            for values in targets:
                definition={'type':'scattered_ore','targets':values,'size':size,'discard_chance_on_air_exposure':chance}
                for legacy in [False,True]:
                    for seed in [0,-1,9223372036854775807]:
                        for mode in range(8):
                            for fail,reject in [(-1,0),(-1,3),(0,0),(7,0)]:
                                yield fixture(definition,legacy,seed,mode,fail,reject)
    for field,values in [('size',[-1,65,4294967297,None,False,'1',0.9]),('discard_chance_on_air_exposure',[-0.0,-1e-100,-0.1,1.01,1e100,None,False,'0.5']),('targets',[None,False,{},'stone',[{}]])]:
        for value in values:
            definition={'type':'scattered_ore','targets':targets[1],'size':8,'discard_chance_on_air_exposure':0.5};definition[field]=value
            yield fixture(definition)
    for p in [[2147483647,2147483647,2147483647],[-2147483648,-2147483648,-2147483648]]:
        for legacy in [False,True]:
            yield fixture({'type':'scattered_ore','targets':targets[1],'size':64,'discard_chance_on_air_exposure':1},legacy=legacy,origin=p)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-scattered-ore-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaScatteredOreOracle.java"
    helper=oracle.with_name("VanillaBlockStateCodecOracle.java")
    tag_helper=oracle.with_name("VanillaHeightmapOracle.java")
    tags=worldgen.tag_resources(jars['server'])
    if not tags or tags!=worldgen.tag_resources(jars['client']):raise ValueError('client/server tags differ')
    (build/'tags.json').write_text(json.dumps(tags,separators=(',',':')),encoding='utf-8')
    sources=[oracle,helper,tag_helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/scattered_ore_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixture_paths=[build/"tags.json"];classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper,tag_helper],"compile-java")
    for side,jar in jars.items():
        path=build/f"facts-{side}.json"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaScatteredOreOracle","export",path],side+"-export")
        fixture_paths.append(path)
    if json.loads((build/"facts-client.json").read_text())!=json.loads((build/"facts-server.json").read_text()):raise ValueError("reference feature block states differ")
    facts=json.loads((build/"facts-server.json").read_text())
    if len(facts['states'])!=35723:raise ValueError('incomplete pinned registry')
    fixtures=list(corpus(facts));inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths.append(inputs)
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"scattered_ore_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaScatteredOreOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} scattered-ore output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"block_states":len(facts["states"]),"integer_properties":len(facts["integers"]),"requested_placements":sum(f["repeats"] for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual scattered_ore feature codec and native ScatteredOreFeature.place. Attempt counts, six float draws per offset, Java rounding, wrapped coordinates, state capture per attempt, ordered target predicates, conditional air-exposure draws, directional neighbor reads, early air detection and mutation flags/results/errors are compared. Both random modes, size/discard boundaries, empty targets, repeated calls, solid/mixed/directional-air worlds and partial failures are included. Complete ore veins, region side effects, chunk decoration and gameplay remain pending."}


    lab.write_json(report_path,report);print(f"Scoped scattered-ore parity passed: {report_path}")


if __name__=="__main__":main()
