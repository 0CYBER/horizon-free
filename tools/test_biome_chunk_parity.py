#!/usr/bin/env python3
"""Compare quart-biome chunk storage and section-atomic filling with pinned vanilla."""
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
    rng=random.Random(0x484f52495a4f4e42494f4348)
    for minimum,height in [(-64,384),(0,128),(-2032,4064),(2000,32),(-16,16)]:
        for cx,cz in [(0,0),(-1,2),(1875000,-1875000),(-2**31,2**31-1)]:
            maximum=minimum+height-1
            queries=[[x,y,z] for x,y,z in [(cx*4,minimum//4,cz*4),(cx*4+3,maximum//4,cz*4+3),(-2**31,-2**31,2**31-1),(2**31-1,2**31-1,-2**31)]]
            queries=[[((v+2**31)%2**32)-2**31 for v in q] for q in queries]
            queries += [[rng.randrange(-2**31,2**31) for _ in range(3)] for _ in range(128)]
            ops=[[0,1,-1,minimum,maximum],[17,67,-1,minimum,maximum],[97,129,0,minimum,maximum],[5,2,63,minimum,maximum],[9,8,64,minimum,maximum],[123,512,65,minimum,maximum],[17,3,-1,minimum+1,maximum-1],[-2**31,67,-1,minimum,maximum]]
            yield {'cx':cx,'cz':cz,'min':minimum,'height':height,'default':rng.randrange(512),'ops':ops,'queries':queries}

    # Retention uses an existing column at quart Y=0. Include fills that overlap
    # that column's section so later sections observe earlier committed sections.
    for minimum,height in [(-64,128),(-64,64),(-16,32),(0,32),(64,16)]:
        for cx,cz in [(0,0),(-1,2),(1875000,-1875000),(-2**31,2**31-1)]:
            maximum=minimum+height-1
            queries=[[-2**31,-2**31,2**31-1],[2**31-1,2**31-1,-2**31],[-1,0,1],[0,-16,0],[3,15,3]]
            ops=[[17,67,-1,minimum,maximum]]
            for before,after in [(False,False),(True,True),(False,True),(True,False)]:
                for salt,count,fail in [(19,512,-1),(0,3,-1),(7,512,0),(9,512,63),(17,512,64),(23,512,65)]:
                    ops.append([salt,count,fail,minimum,maximum,before,after])
                ops.append([31,512,-1,minimum+1,maximum-1,before,after])
            resolver_queries=[[candidate,*q,before,after] for candidate in [0,1,2,3,4,511,-1]
                              for before,after in [(False,False),(False,True),(True,False),(True,True)] for q in queries]
            yield {'cx':cx,'cz':cz,'min':minimum,'height':height,'default':4,'ops':ops,'queries':queries,'resolver_queries':resolver_queries}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-biome-chunk-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomeChunkOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/biome_chunk_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"biome_chunk_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeChunkOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} biome-chunk output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"fills":sum(len(f["ops"]) for f in fixtures),"block_queries":sum(len(f["ops"])*len(f["queries"]) for f in fixtures),"retrogen_fills":sum(len(op)>5 for f in fixtures for op in f["ops"]),"resolver_queries":sum(len(f.get("resolver_queries",[])) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual ChunkAccess.fillBiomesFromNoise, LevelChunkSection.fillBiomesFromNoise and chunk/section biome reads using synthetic registered holder identities. Ordered resolver queries, section contents and clamped/wrapped chunk lookups are compared after repeated fills and failures at section boundaries. Covers full supported aligned vertical bounds, generation bounds cutting through sections, negative/extreme chunk coordinates and 1..512 possible biome identities. Completed earlier sections survive failure while the failing section retains its previous values. Additional BelowZeroRetrogen.getBiomeResolver cases compare all three retained cave keys, a lookalike foreign namespace, ordinary keyed holders and a direct holder without a key. All four combinations of upgrade activation at resolver construction and after construction test the captured wrapping decision. Candidate invocation order, injected candidate failures, old-column reads with quart Y fixed to zero and X/Z wrapping, repeated fills, and overlap of generation bounds with the old column section are exercised. Synthetic generation bounds deliberately override the fixed native retrogen height to expose section commit/read ordering. Direct holder values and all keyed holders are identity-only placeholders; biome properties are not evaluated. This compares stored biome values and resolver behavior, not native in-memory palette layout, wire encoding, biome-source generation integration, nonempty blending or complete generated worlds."}

    lab.write_json(report_path,report);print(f"Scoped biome-chunk parity passed: {report_path}")


if __name__=="__main__":main()
