#!/usr/bin/env python3
"""Compare below-zero upgrade lifecycle, bedrock replacement and hole clearing."""
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
    defaults=facts['defaults']
    stages=['empty','structure_starts','structure_references','biomes','terrain','features']
    targets=stages[1:]+['initialize_light','light','spawn','full']
    states=[defaults['minecraft:'+name] for name in ['bedrock','deepslate','stone','air','water','cave_air','void_air','oak_leaves','oak_slab','snow','powder_snow','lava']]
    masks=[[],[1],[1,-9223372036854775808,0,9223372036854775807],[-1]*4,[0,0,0,0,1]]
    index=0
    for minimum,height in [(-64,80),(0,16),(-16,32),(64,16)]:
        for stage in ['empty','terrain']:
            for upgrading in [False,True]:
                for words in masks:
                    ops=[['stage',stage]]
                    if index%2==0:ops.append(['prime',63])
                    ops += [['fill',states,index%len(states)],['dump'],['upgrade','features' if upgrading else None,[1]],['upgrade_info'],['replace_bedrock'],['replace_bedrock'],['dump'],['mask',words],['mask',words],['dump'],['stage','features'],['upgrade_info'],['mask',words],['dump']]
                    yield {'chunk':[0,0] if index%2 else [-3,7],'min':minimum,'height':height,'ops':ops}
                    index+=1
        yield {'chunk':[-3,7],'min':minimum,'height':height,'ops':[['fill',[defaults['minecraft:air']],0],['upgrade','terrain',[-1]*4],['replace_bedrock'],['mask',[-1]*4],['dump'],['upgrade_info']]}
    for initial in stages:
        for target in targets:
            ops=[['stage',initial],['upgrade',target,[1]],['upgrade_info'],['stage',initial],['upgrade_info'],['upgrade',target,[1]],['upgrade_info']]
            for stage in stages+list(reversed(stages)):ops += [['stage',stage],['upgrade_info']]
            ops += [['upgrade',target,[1]],['upgrade_info'],['upgrade',None,[]],['upgrade_info']]
            yield {'chunk':[2147483647,-2147483648],'min':0,'height':16,'ops':ops}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-retrogen-chunk-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaChunkStageOracle.java"
    helper=oracle.with_name("VanillaHeightmapOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/chunk_stage_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity.VanillaChunkStageOracle",mode,path],side+"-"+mode)
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
    run([binary,"chunk_stage_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} retrogen-chunk output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"operation_counts":dict(Counter(op[0] for fixture in fixtures for op in fixture["ops"])),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Native ProtoChunk below-zero metadata installation/removal, pre-light persisted-stage assignments and target completion cleanup, normal/upgrading generation heights, highest-generated status from retained targets versus current persisted stages, actual BelowZeroRetrogen.replaceOldBedrock and applyBedrockMask. Mixed resident states, legacy five-layer bedrock bands, signed/chunk-relative coordinates, partial vertical overlap, masks with empty/single/sparse/all/out-of-footprint bits, repeated operations, preprimed/stale/missing heightmaps, and clearing under changed generation bounds are compared. Complete raw sections and selected packed heightmaps are checked after each mutation phase. All nine nonempty targets are paired with all six pre-light stages, including installation after an already-advanced status, repeated assignments, backward transitions and explicit removal. Data codecs have a separate comparison. Save-dirty notifications, post-light stages, structure restrictions, biome retention, automatic generation scheduling and complete world/gameplay parity remain outside this comparison."}

    lab.write_json(report_path,report);print(f"Scoped retrogen-chunk parity passed: {report_path}")


if __name__=="__main__":main()
