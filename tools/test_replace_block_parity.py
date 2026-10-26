#!/usr/bin/env python3
"""Compare ordered block replacements and rule-driven mutations with pinned vanilla."""
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
    rules=[yes,no,{'predicate_type':'tag_match','tag':'leaves'}, {'predicate_type':'tag_match','tag':'horizon:missing'}, {'predicate_type':'height_match','min_inclusive':-64,'max_inclusive':63}]
    for name in ['air','cave_air','void_air','stone','water','oak_leaves','dirt']:
        for kind,field in [('block_match','block'),('blockstate_match','block_state')]:
            rules.append({'predicate_type':kind,field:name})
        rules += [{'predicate_type':'random_block_match','block':name,'probability':p} for p in [0,0.5,1,2]]
    rules += [{'predicate_type':'any_of','rules':rules[5:12]}, {'predicate_type':'all_of','rules':[no,yes]}]
    definitions=[{'type':'replace_single_block','targets':[]}]
    for rule in rules:
        targets=[{'target':rule,'state':{'id':'oak_log','properties':{'axis':'x'}}}]
        definitions += [{'type':'replace_single_block','targets':targets}, {'type':'replace_single_block','targets':[{'target':no,'state':'air'},*targets,{'target':yes,'state':'water'}]}]
    for value in [None,False,'stone',{},[{}],[{'target':yes,'state':'missing'}],[{'target':yes,'state':'stone'},{'target':{'predicate_type':'missing'},'state':'air'}]]:
        definitions.append({'type':'replace_single_block','targets':value})
    for legacy in [False,True]:
        for definition in definitions:
            for x in range(7):
                for fail in [-1,0,1,2,4]:
                    for reject in [0,2]:
                        yield {'definition':json.dumps(definition,separators=(',',':')),'origin':[x,-64+x*32,0],'world_min':-64,'legacy':legacy,'seed':-1 if legacy else 0,'reject':reject,'fail':fail,'repeats':3}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-replace-block-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaReplaceBlockOracle.java"
    helper=oracle.with_name("VanillaBlockStateCodecOracle.java")
    tag_helper=oracle.with_name("VanillaHeightmapOracle.java")
    tags=worldgen.tag_resources(jars['server'])
    if not tags or tags!=worldgen.tag_resources(jars['client']):raise ValueError('client/server tags differ')
    (build/'tags.json').write_text(json.dumps(tags,separators=(',',':')),encoding='utf-8')
    sources=[oracle,helper,tag_helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/replace_block_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixture_paths=[build/"tags.json"];classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper,tag_helper],"compile-java")
    for side,jar in jars.items():
        path=build/f"facts-{side}.json"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaReplaceBlockOracle","export",path],side+"-export")
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
    run([binary,"replace_block_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaReplaceBlockOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} replace-block output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"block_states":len(facts["states"]),"integer_properties":len(facts["integers"]),"requested_placements":sum(f["repeats"] for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual replace_single_block feature codecs and native ReplaceBlockFeature.place. Ordered target rules re-read the origin state, short-circuit on the first match, preserve random draw order and write flags, ignore rejected writes, retain partial mutations and stop on injected read/write failures. Bound block/tag/state/height/random/logical targets and malformed unselected entries are compared across both random modes and repeated calls. Full region side effects, structure processors, ore clusters and complete gameplay remain pending."}


    lab.write_json(report_path,report);print(f"Scoped replace-block parity passed: {report_path}")


if __name__=="__main__":main()
