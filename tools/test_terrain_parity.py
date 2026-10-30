#!/usr/bin/env python3
"""Compare noise-stage terrain filling with pinned vanilla client/server bytecode."""
import argparse
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


def corpus(facts,bundle):
    defaults=facts["defaults"]
    rng=random.Random(0x484f52495a4f4e46494c4c)
    def fixture(minimum,height,cx,cz,seed,legacy,block,fluid,sea,router,aquifers,repeats=1):
        return {"min":minimum,"height":height,"noise_min":minimum,"noise_height":height,"cx":cx,"cz":cz,"seed":seed,"legacy":legacy,
                "block":block,"fluid":fluid,"sea":sea,"router":router,"aquifers":aquifers,"initial":[],"repeats":repeats}
    for setting,text in bundle["settings"].items():
        s=json.loads(text)
        for legacy in [False,True]:
            for seed,cx,cz in [(0,0,0),(-1,-2,3),(0x123456789abcdef,1875000,-1875000)]:
                f=fixture(s["noise"]["min_y"],s["noise"]["height"],cx,cz,seed,legacy,defaults[s["default_block"]],defaults[s["default_fluid"]],s["sea_level"],s["noise_router"],s.get("aquifers"),2 if seed==0 else 1)
                for i in range(16):f["initial"].append([i,f["min"]+(i*17)%f["height"],15-i,defaults["minecraft:glass"]])
                yield f
    fields=["temperature","vegetation","continents","erosion","depth","ridges","chunk_surface_level","final_density"]
    config={"barrier":0,"fluid_level_floodedness":1,"fluid_level_spread":0,"lava":0,"exclusion":0,"surface_level":64}
    wet_slab=next(i for i,state in enumerate(facts["states"]) if state[0]=="minecraft:oak_slab" and state[2])
    states=[defaults["minecraft:"+name] for name in ["air","cave_air","void_air","stone","water","lava","oak_leaves"]]+[wet_slab]
    for scenario in range(48):
        router=dict.fromkeys(fields,0)
        router["final_density"]=[0,1,-1,{"type":"gradient","axis":"y","from_coordinate":-64,"to_coordinate":0,"from_value":1,"to_value":-1}][scenario%4]
        f=fixture(-64,64,scenario%5-2,scenario//5-4,rng.randrange(-2**63,2**63),bool(scenario%2),states[(scenario//4)%len(states)],states[scenario%len(states)],[-54,-20,0,64][scenario%4],router,config if scenario%3 else None,2)
        # Fill only the middle of an extended chunk; prefilled areas must survive.
        if scenario%5==0:f.update(min=-80,height=96)
        f["initial"]=[[rng.randrange(16),f["min"]+rng.randrange(f["height"]),rng.randrange(16),rng.choice(states)] for _ in range(48)]
        yield f


    # Real structure-density bindings through both aquifer and final-density roots.
    for _,text in bundle["settings"].items():
        setting=json.loads(text)
        for legacy in [False,True]:
            y=max(setting["noise"]["min_y"]+16,min(setting["sea_level"],setting["noise"]["min_y"]+setting["noise"]["height"]-16))
            f=fixture(setting["noise"]["min_y"],setting["noise"]["height"],0,0,123,legacy,defaults[setting["default_block"]],defaults[setting["default_fluid"]],setting["sea_level"],setting["noise_router"],setting.get("aquifers"),2)
            f["initial"]=[[1,y,1,defaults["minecraft:glass"]]]
            f["structure"]={"pieces":[[[2,y-4,2,10,y+8,10],"BEARD_BOX",2],[[8,y-2,8,15,y+2,15],"BURY",0]],"junctions":[[3,y,4],[11,y+1,13]],"affected":[-32,y-32,-32,48,y+40,48]}
            yield f
    for kind in ["NONE","BURY","BEARD_THIN","BEARD_BOX","ENCAPSULATE"]:
        for enabled in [False,True]:
            router=dict.fromkeys(fields,0);router["final_density"]={"type":"cache","input":{"type":"beardifier"}}
            aq=dict(config);aq["barrier"]={"type":"beardifier"};aq["surface_level"]={"type":"add","left":-16,"right":{"type":"beardifier"}}
            f=fixture(-64,96,-1,1,17,False,defaults["minecraft:stone"],defaults["minecraft:water"],-16,router,aq if enabled else None,2)
            f["initial"]=[[0,0,0,defaults["minecraft:glass"]]]
            f["structure"]={"pieces":[[[-14,-24,18,-3,-8,28],kind,3]],"junctions":[[-6,-18,23]],"affected":[-40,-48,-8,16,24,56]}
            yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args=parser.parse_args()
    build=lab.LOCAL/"rust-terrain-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    tags=worldgen.tag_resources(jars["server"])
    if not tags or tags!=worldgen.tag_resources(jars["client"]):raise ValueError("client/server block tags differ or are empty")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaTerrainOracle.java"
    exporter=oracle.with_name("VanillaHeightmapOracle.java")
    sources=[oracle,exporter,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",
             lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/terrain_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,exporter],"compile-java")
    def java(side,mode,path):
        run(["java","-Xmx1G","-cp",os.pathsep.join([str(classes),str(jars[side]),dependency_path]),"dev.horizonlab.parity."+("VanillaHeightmapOracle" if mode=="export" else "VanillaTerrainOracle"),mode,path],side+"-"+mode)
    for side in jars:java(side,"export",build/f"facts-{side}.json")
    facts=json.loads((build/"facts-server.json").read_text())
    if facts!=json.loads((build/"facts-client.json").read_text()):raise ValueError("client/server state facts differ")
    if len(facts["states"])!=35723:raise ValueError("incomplete pinned state table")
    bundle={"tags":tags}
    for field,kind in [("noises","noise"),("densities","density_function"),("settings","noise_settings")]:
        resources=worldgen.registry_resources(jars["server"],kind)
        if not resources or resources!=worldgen.registry_resources(jars["client"],kind):raise ValueError("client/server "+kind+" resources differ")
        prefix="data/minecraft/worldgen/"+kind+"/"
        bundle[field]={"minecraft:"+name[len(prefix):-5]:text for name,text in resources.items()}
    fixtures=list(corpus(facts,bundle));inputs=build/"inputs.txt"
    inputs.write_text("".join(json.dumps(value,separators=(",",":"))+"\n" for value in fixtures),encoding="utf-8")
    (build/"registries.json").write_text(json.dumps(bundle,separators=(",",":")),encoding="utf-8")
    fixture_paths=[inputs,build/"registries.json",build/"facts-client.json",build/"facts-server.json"]
    tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"terrain_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        java(side,side,build)
        reference=build/f"{side}.txt";matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} terrain-fill output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "block_states":len(facts["states"]),"block_tag_files":len(tags),"fixtures":len(fixtures),"fill_calls":sum(f["repeats"] for f in fixtures),"noise_aquifer_fixtures":sum(f["aquifers"] is not None for f in fixtures),"structure_density_fixtures":sum("structure" in f for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Noise-stage filling only, using actual vanilla NoiseBasedChunkGenerator.doFill and NoiseChunk with empty blending and empty or supplied rigid-piece/junction structure fields. Independently authenticated client/server registry resources and exported block identities/properties/air/fluid facts must agree. Rust compiles the final density and optional aquifer through a shared compiler/context, writes resident block sections, updates ocean-floor/world-surface worldgen heightmaps and records ordered per-section fluid postprocessing offsets. Comparisons cover every raw section state, heightmap word and postprocessing offset after each fill, all bundled routers/aquifers, both seeded random factories, synthetic roots, prefilled chunks, partial vertical volumes, repeated fills and structure-density bindings in final-density/aquifer graphs. Block behavior facts remain imported; actual biome properties and material rules are not evaluated. Surface/material/carver/feature/structure stages, nonempty blending and live structure discovery, complete generated worlds and gameplay/rendering remain pending."}
    lab.write_json(report_path,report);print(f"Scoped terrain-fill parity passed: {report_path}")


if __name__=="__main__":main()
