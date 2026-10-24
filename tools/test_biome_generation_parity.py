#!/usr/bin/env python3
"""Compare chunk generation for all four biome-source kinds with pinned vanilla."""
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


def corpus(bundle):
    for setting,raw in sorted(bundle['settings'].items()):
        value=json.loads(raw);minimum=value['noise']['min_y'];height=value['noise']['height']
        for preset in sorted(bundle['presets']):
            for legacy in [False,True]:
                for seed,cx,cz in [(0,0,0),(-1,-2,3),(0x123456789abcdef,1875000,-1875000)]:
                    queries=[[cx*16+x,y,cz*16+z] for x in [-1,0,2,7,15,16] for z in [-1,0,2,7,15,16] for y in [minimum-20,minimum,minimum+17,minimum+height-1,minimum+height+20]]
                    yield {'setting':setting,'preset':preset,'seed':seed,'legacy':legacy,'cx':cx,'cz':cz,'min':minimum,'height':height,'write_min':minimum+(17 if seed==-1 else 0),'write_max':minimum+height-1-(17 if seed==-1 else 0),'queries':queries,'repeats':2 if seed==0 else 1}


        for source in ['fixed','checkerboard','end']:
            for legacy in [False,True]:
                for seed,cx,cz,scale in [(0,0,0,2),(-1,65,0,0),(2,-65,1,30),(-2**63,1875000,-1875000,62)]:
                    queries=[[cx*16+x,y,cz*16+z] for x in [-1,0,7,15,16] for z in [-1,0,7,15,16] for y in [minimum-1,minimum+17,minimum+height]]
                    yield {'setting':setting,'preset':'minecraft:overworld','source':source,'scale':scale,'seed':seed,'legacy':legacy,'cx':cx,'cz':cz,'min':minimum,'height':height,'write_min':minimum,'write_max':minimum+height-1,'queries':queries,'repeats':2 if seed==0 else 1}

    for setting,raw in sorted(bundle['settings'].items()):
        value=json.loads(raw);minimum=value['noise']['min_y'];height=value['noise']['height']
        if minimum != -64: continue
        for legacy in [False,True]:
            for mode in ['multi_overworld','multi_nether','fixed','cave_fixed','checkerboard','end']:
                for sample,(seed,cx,cz) in enumerate([(0,0,0),(-1,-2,3),(0x123456789abcdef,1875000,-1875000)]):
                    queries=[[cx*16+x,y,cz*16+z] for x in [-1,0,7,15,16] for z in [-1,0,7,15,16] for y in [-65,-64,-17,-1,0,1,minimum+height-1,minimum+height]]
                    fixture={'setting':setting,'preset':'minecraft:nether' if mode=='multi_nether' else 'minecraft:overworld','seed':seed,'legacy':legacy,'cx':cx,'cz':cz,'min':minimum,'height':height,'write_min':-64,'write_max':-1,'retrogen':True,'queries':queries,'repeats':2,'scale':2}
                    if not mode.startswith('multi_'):fixture['source']='fixed' if mode=='cave_fixed' else mode
                    if mode=='cave_fixed':fixture['fixed_biome']=['minecraft:lush_caves','minecraft:dripstone_caves','minecraft:deep_dark'][sample]
                    yield fixture


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-biome-generation-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomeGenerationOracle.java"
    exporter=oracle.with_name("VanillaBiomeExport.java")
    sources=[oracle,exporter,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/biome_generation_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,exporter],"compile-java")
    bundle={}
    for field,kind in [('noises','noise'),('densities','density_function'),('settings','noise_settings')]:
        resources=worldgen.registry_resources(jars['server'],kind)
        if not resources or resources!=worldgen.registry_resources(jars['client'],kind):raise ValueError('reference '+kind+' resources differ')
        prefix='data/minecraft/worldgen/'+kind+'/'
        bundle[field]={'minecraft:'+name[len(prefix):-5]:raw for name,raw in resources.items()}
    exports=[]
    for side,jar in jars.items():
        path=build/f'presets-{side}.json';exports.append(path)
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaBiomeExport',path],side+'-export')
        presets=json.loads(path.read_text())
        if not presets or ('presets' in bundle and presets!=bundle['presets']):raise ValueError('reference biome preset tables differ')
        bundle['presets']=presets
    registries=build/'registries.json';registries.write_text(json.dumps(bundle,separators=(',',':')),encoding='utf-8')
    fixtures=list(corpus(bundle));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(f,separators=(',',':'))+'\n' for f in fixtures),encoding='utf-8')
    fixture_paths=[inputs,registries,*exports];tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"biome_generation_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeGenerationOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} biome-generation output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"retrogen_fixtures":sum(f.get("retrogen",False) for f in fixtures),"fills":sum(f["repeats"] for f in fixtures),"biome_values":sum(f["repeats"]*f["height"]*4 for f in fixtures),"block_queries":sum(f["repeats"]*len(f["queries"]) for f in fixtures),"preset_entries":{name:len(json.loads(raw)) for name,raw in bundle["presets"].items()},"source_fixtures":{name:sum(f.get("source","multi")==name for f in fixtures) for name in ["multi","fixed","checkerboard","end"]},
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual ChunkGenerator.doCreateBiomes through NoiseBasedChunkGenerator with empty Blender, in ordinary and below-zero upgrade modes. Both bundled Overworld/Nether multi-noise preset tables are independently exported from authenticated JARs; Rust imports quantized tables and compiles climate routers itself. All seven bundled routers, both random modes, multiple seeds and distant positive/negative chunks are tested. Complete section biome values and subsequent BiomeManager block lookups are compared, including repeated generation with fresh density contexts and persistent previous-leaf search state, and generation bounds excluding top/bottom sections. Holders are identity placeholders; no biome properties are evaluated. Fixed, checkerboard and End source fixtures also run actual chunk generation through every bundled router, including real seeded End erosion, the central-island shortcut, distant coordinates and wrapped checkerboard shifts. Retrogen cases initialize patterned old biome columns, install actual below-zero metadata with a full target on an empty persisted-stage ProtoChunk, and use its native generation height. Four below-zero noise settings, all source kinds, fixed candidates for each of the three retained cave biomes, both random modes, signed/distant seeds and repeated fills are compared. Above-zero stored biomes remain checked alongside regenerated sections and block-level biome zoom. Rust composes seeded candidate resolvers with the independently tested retention fill. Preset builders, complete biome-source codecs, nonempty blending, complete terrain-stage scheduling and complete generated worlds remain pending."}

    lab.write_json(report_path,report);print(f"Scoped biome-generation parity passed: {report_path}")


if __name__=="__main__":main()
