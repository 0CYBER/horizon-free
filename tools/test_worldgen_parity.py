#!/usr/bin/env python3
"""Compare world-generation JSON decoding with the pinned vanilla codecs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from collections import Counter
import platform
import random
import subprocess
import zipfile

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(resources):
    rng = random.Random(0x484f52495a4f4e4a534f4e)
    seeds = [0,1,-1,-2**63,2**63-1,0x123456789abcdef]
    for text in resources.values():
        compact = " ".join(text.splitlines())
        for seed in seeds:
            for legacy,nether in [(0,0),(1,0),(0,1),(1,1)]:
                yield f"noise {seed} {legacy} {nether} {compact}"
    fields = {
        "base_octave": ["-33","-32","0","32","33","4294967295","4294967296","-4294967297.9","1.9999999999999999999","12e1","123e-1","-123e-3","123e99","null","true",'"-7"'],
        "octave_count": ["0","1","32","33","-1","null","false","1.9999999999999999999","4294967297"],
        "base_amplitude": ["0","0.000009999999747378751","0.000009999999747378752","1","1000000","1000000.0000000001","null","true"],
        "normalize": ["true","false",'"legacy"',"0","1","0.5","4294967296","null",'"true"','"LEGACY"',"{}"],
        "amplitude_modifiers": ["[]","[0]","[-0]","[-0.0]","[1]","[1000000]","[1000001]","[-1]","[null]","null","[1,2]","{}"],
    }
    for name,values in fields.items():
        for value in values:
            text = '{"base_octave":-7,"'+name+'":'+value+'}' if name != "base_octave" else '{"base_octave":'+value+'}'
            yield f"noise 0 0 0 {text}"
    for text in ['{}','[]','null','42','true','{"base_octave":-7,"unknown":{}}','{"base_octave":-7,"base_octave":-8}']:
        yield f"noise 0 0 0 {text}"
    for scenario in range(512):
        count = 1 + scenario % 8
        parameters = {"base_octave":rng.randrange(-16,1),"octave_count":count}
        if scenario % 3:
            parameters["base_amplitude"] = rng.choice([.00001,.125,1.,1e6,rng.uniform(.00001,1e6)])
        if scenario % 4:
            parameters["normalize"] = [True,False,"legacy"][scenario % 3]
        if scenario % 5:
            parameters["amplitude_modifiers"] = [rng.choice([0.,.125,1.,1e6,rng.uniform(0,1e6)]) for _ in range(count)]
        yield f"noise {rng.randrange(-2**63,2**63)} {(scenario//2)%2} {scenario%2} "+json.dumps(parameters,separators=(",",":"))


def registry_resources(jar, kind):
    prefix = f"data/minecraft/worldgen/{kind}/"
    with zipfile.ZipFile(jar) as archive:
        return {name:archive.read(name).decode("utf-8") for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith(".json")}


def tag_resources(jar):
    prefix="data/minecraft/tags/block/"
    with zipfile.ZipFile(jar) as archive:
        return {"minecraft:"+name[len(prefix):-5]:archive.read(name).decode("utf-8") for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith(".json")}


def tag_corpus(resources):
    entry_values=['"stone"','"#leaves"','""','"#"','":stone"','"minecraft:"','"Minecraft:stone"','"STONE"','"a:b:c"','"a/b:stone"','"#minecraft:leaves"','"##leaves"','42','true','null','[]','{}','{"id":"stone"}','{"id":"#leaves","required":false}','{"id":"stone","required":null}','{"id":null}','{"required":true}']
    for value in entry_values:yield "tag_entry "+value
    for required in ['true','false','null','0','1','0.5','4294967296','"true"','{}','[]']:
        yield 'tag_entry {"id":"stone","required":'+required+'}'
    for text in resources.values():yield "tag_file "+" ".join(text.splitlines())
    for value in entry_values:yield 'tag_file {"values":['+value+']}'
    for value in ['true','false','null','0','1','"false"','{}']:
        yield 'tag_file {"values":[],"replace":'+value+'}'
    for text in ['{}','[]','null','{"values":null}','{"values":true}','{"values":[],"replace":true,"replace":false}']:
        yield 'tag_file '+text
    def command(files,elements):return "tags "+json.dumps({"files":files,"elements":elements},separators=(",",":"))
    def file(name,values,replace=False):return [name,"fixture",json.dumps({"values":values,"replace":replace},separators=(",",":"))]
    elements={"minecraft:a":"a","minecraft:b":"b","minecraft:c":"c","minecraft:alias":"a"}
    yield command([file("first",["b","a","b","alias"]),file("second",["#first","c","a"])],elements)
    yield command([file("a",["a","b"]),file("a",["c"],True),file("a",["a"]),file("empty",[])],elements)
    yield command([["a","broken","{}"],file("a",["a"]),["a","broken","{\"replace\":true,\"values\":[true]}"],["empty","broken","null"]],elements)
    yield command([file("a",["missing","a"]),file("b",["#a","b"]),file("c",[{"id":"#a","required":False},"c"])],elements)
    for mandatory in [True,False]:
        yield command([file("a",[{"id":"#b","required":mandatory},"a"]),file("b",[{"id":"#a","required":mandatory},"b"])],elements)
    # Required dependencies are processed globally before optional ones.
    yield command([file("a",[{"id":"#b","required":False},"a"]),file("b",["#a","b"])],elements)
    # Canonicalize unordered missing-tag output by full string, not Identifier's
    # path-first comparator; lookup/dependency iteration remains observable.
    yield command([file("z:first",["missing"]),file("a:last",["#z:first"]),file("example:child",[{"id":"#z:first","required":False},"a"])],elements)
    rng=random.Random(0x484f52495a4f4e54414753)
    for scenario in range(192):
        count=1+scenario%48;files=[]
        for i in range(count):
            values=[]
            for j in range(rng.randrange(9)):
                if i and j%3==0:
                    # Later dependency names make arbitrary cycles possible.
                    child=rng.randrange(count) if scenario%4==0 else rng.randrange(i)
                    if child==i:child=(child+1)%count
                    values.append({"id":f"#tag_{child}","required":j%2==0})
                else:values.append({"id":rng.choice(["a","b","c","alias","missing"]),"required":j%3!=0})
            files.append(file(f"tag_{i}",values))
            if i%11==0:files.append(file(f"tag_{i}",["c"],scenario%3==0))
        yield command(files,elements)


def noise_resources(jar):
    return registry_resources(jar,"noise")


def climate_corpus():
    rng = random.Random(0x484f52495a4f4e434c494d)
    edges = [0,0x80000000,1,0x80000001,0x007fffff,0x00800000,
             0x3f800000,0xbf800000,0x38d1b717,0xb8d1b717,
             0x58612e12,0xd8612e12,0x7f7fffff,0xff7fffff,
             0x7f800000,0xff800000,0x7f800001,0xff800001,0x7fc00000,0xffc00000]
    longs = [0,1,-1,-2**63,2**63-1,-10000,10000,16777217,-16777217]
    for scenario in range(len(edges)+4096):
        values = [edges[(scenario+i)%len(edges)] for i in range(6)] if scenario < len(edges) else [rng.getrandbits(32) for _ in range(6)]
        value = longs[scenario%len(longs)] if scenario < len(edges) else rng.randrange(-2**63,2**63)
        yield "quantize "+" ".join(f"{value:08x}" for value in values)+f" {value}"


def climate_codec_corpus():
    values = ["-2.0000002","-2","-0.0001","-0","0","0.0001","1","2","2.0000001","2.0000001192092895507812500000000001","2.0000002","1e100","null","true",'"1"',"[]","[0]","[0,0,0]","[-2,2]","[1,-1]","[-0,0]","[0,-0]",'{"min":-2,"max":2}','{"min":0}', '{"min":0,"max":0,"ignored":1}']
    for value in values:
        yield "climate_parameter "+value
    fields = ["temperature","humidity","continentalness","erosion","depth","weirdness","offset"]
    for field in fields:
        for value in values:
            yield "climate_point {"+",".join('"'+name+'":'+(value if field == name else "0") for name in fields)+"}"
        yield "climate_point {"+",".join('"'+name+'":0' for name in fields if field != name)+"}"
    yield 'climate_point {"temperature":0,"humidity":0,"continentalness":0,"erosion":0,"depth":0,"weirdness":0,"offset":0,"offset":1}'


def settings_corpus(resources):
    for text in resources.values():
        # The noise member contains integers in bundled resources. Keep all
        # synthetic numeric tokens below raw for Gson integer conversion cases.
        yield "noise_settings "+json.dumps(json.loads(text)["noise"],separators=(",",":"))
    minima = [-2**31,-2033,-2032,-2031,-65,-64,-16,-1,0,1,16,2031,2032,2**31-1]
    heights = [-2**31,-16,-1,0,1,15,16,128,384,4063,4064,4065,2**31-1]
    for minimum in minima:
        for height in heights:
            yield f'noise_settings {{"min_y":{minimum},"height":{height}}}'
            yield f"settings_create {minimum} {height}"
    for field in ["min_y","height"]:
        for value in ["null","true",'"16"',"4294967296","4294967312","-4294967312.999999999999","15.9999999999999999999","16.9999999999999999999","16e0","16e-1","16e99"]:
            yield 'noise_settings {"'+field+'":'+value+','+('"height":16' if field == "min_y" else '"min_y":0')+'}'
    for text in ['{}','[]','null','{"min_y":0}','{"height":0}','{"min_y":0,"height":16,"height":32,"ignored":0}']:
        yield "noise_settings "+text


def aquifer_corpus():
    rng = random.Random(0x484f52495a4f4e464c554944)
    bits = [0,0x8000000000000000,1,0x8000000000000001,0x3ff0000000000000,0xbff0000000000000,0x7fefffffffffffff,0xffefffffffffffff,0x7ff0000000000000,0xfff0000000000000,0x7ff0000000000001,0xfff8000000000000]
    for sea in [-2**31,-64,-54,0,63,2**31-1]:
        for fluid in range(5):
            for y in [-2**31,-4065,-4064,-65,-64,-55,-54,-53,-1,0,62,63,64,2**31-1]:
                for value in bits:
                    yield f"aquifer {sea} {fluid} 0 {y} 0 {value:016x}"
    for _ in range(4096):
        sea,x,y,z = [rng.randrange(-2**31,2**31) for _ in range(4)]
        yield f"aquifer {sea} {rng.randrange(5)} {x} {y} {z} {rng.getrandbits(64):016x}"


def noise_aquifer_corpus(bundle, resources):
    """Stateful original transport, with vanilla samplers and fault injection."""
    rng = random.Random(0x484f52495a4f4e41515549)
    density_bits = ["0000000000000000","8000000000000000","bff0000000000000","bfb999999999999a","0000000000000001","8000000000000001","7ff0000000000000","fff0000000000000","7ff0000000000001","fff8000000000000"]
    def emit(seed, legacy, setting, fixture):
        return f"noise_aquifer {seed} {legacy} {bundle} {setting} "+json.dumps(fixture,separators=(",",":"))
    def fixture(minimum, sizes=(16,128,16), steps=(1,1,1), count=128):
        queries=[]
        for i in range(count):
            # Exercise strided volume extents, not just density lattice samples.
            p=[minimum[a]+rng.randrange(sizes[a]*steps[a]) for a in range(3)]
            queries.append([*p,density_bits[i%len(density_bits)]])
        queries += queries[:16] + list(reversed(queries[:16]))
        return {"sizes":sizes,"minimum":minimum,"steps":steps,"sea":63,"fluid":0,"queries":queries}
    for setting in sorted(resources):
        for seed in [0,1,-1,-2**63,2**63-1,0x123456789abcdef]:
            for legacy in [0,1]:
                f=fixture([-16,-64,32])
                yield emit(seed,legacy,setting,f)
    def gradient(axis, low, high, start, end):
        return {"type":"gradient","axis":axis,"from_coordinate":low,"to_coordinate":high,"from_value":start,"to_value":end}
    base={"barrier":0,"fluid_level_floodedness":gradient("x",-96,96,-1,1),"fluid_level_spread":gradient("y",-4,4,-1,1),"lava":gradient("z",-4,4,-1,1),"exclusion":0,"surface_level":96}
    origins=[[0,-64,0],[-16,-80,32],[33554416,-64,-33554432],[2147483600,-64,-2147483600]]
    float_bits=["00000000","80000000","00000001","80000001","3f800000","bf800000","3f19999a","3f4ccccd","7f800000","ff800000","7fc00000","7f800001","7f7fffff","ff7fffff"]
    for scenario in range(160):
        f=fixture(origins[scenario%len(origins)],count=192)
        config=dict(base)
        if scenario%5==0: config["fluid_level_floodedness"]=[-1,0,.4,.6,.8,1][scenario%6]
        if scenario%7==0: config["surface_level"]=[-96,-64,-20,0,64,96,256][scenario%7]
        if scenario%9==0: config["exclusion"]=1
        f["config"]=json.dumps(config,separators=(",",":"))
        f["sea"]=[-2**31,-64,-54,-10,0,63,2**31-1][scenario%7]
        f["fluid"]=scenario%8
        if scenario%3==0:
            f["overrides"]=[None]*6
            f["overrides"][(scenario//3)%6]=float_bits[(scenario//3)%len(float_bits)]
        if scenario%11==0:
            f["steps"]=[4,2,4]; f["sizes"]=[4,64,4]
        if scenario%13==0:
            # Preserve the scheduling flag after a failed array access.
            f["queries"] += [[f["minimum"][0]+512,0,f["minimum"][2],"bff0000000000000"]]
        yield emit(rng.randrange(-2**63,2**63),scenario%2,"minecraft:overworld",f)
    for field in range(6):
        for value in float_bits:
            f=fixture([0,-54,0],sizes=(16,64,16),count=128)
            config=dict(base);config["fluid_level_floodedness"]=.6
            config["fluid_level_spread"]=gradient("x",-4,4,-1,1)
            f["config"]=json.dumps(config,separators=(",",":"))
            f["overrides"]=[None]*6;f["overrides"][field]=value
            yield emit(0,field%2,"minecraft:overworld",f)
    # Fail once at each binding's first/second evaluation, then retry the same
    # ordered queries. This also exercises constructor surface-volume failure.
    for field in range(6):
        for occurrence in [1,2,8]:
            f=fixture([0,-54,0],sizes=(16,64,16),count=0)
            config=dict(base);config["fluid_level_floodedness"]=.6
            config["fluid_level_spread"]=gradient("x",-4,4,-1,1)
            f["config"]=json.dumps(config,separators=(",",":"));f["fail"]=[field,occurrence]
            f["queries"]=[[x,y,z,"bfb999999999999a"] for x in range(16) for y in [-40,-30,-20,-10,0] for z in [0,7,15]]
            if field==0:
                f["sizes"]=[32,96,32]
                f["queries"]=[[x,y,z,"bfb999999999999a"] for x in range(32) for y in [-40,-30,-20,-10,0,10,20,30] for z in range(0,32,3)]
            f["queries"] += f["queries"][:64]
            yield emit(0,field%2,"minecraft:overworld",f)
    # Required-field codecs and malformed roots are validated before binding.
    configs=["null","[]","{}"]
    for field in base:
        for value in [None,True,{},"minecraft:missing"]:
            config=dict(base);config[field]=value;configs.append(json.dumps(config,separators=(",",":")))
        config=dict(base);del config[field];configs.append(json.dumps(config,separators=(",",":")))
    for config in configs:
        f=fixture([0,-64,0],count=0);f["config"]=config
        yield emit(0,0,"minecraft:overworld",f)


def index_corpus():
    rng = random.Random(0x484f52495a4f4e54524545)
    def command(branching, entries, targets):
        return f"index {branching} "+json.dumps({"entries":entries,"targets":targets},separators=(",",":"))
    def entry(coordinates, offset=0):
        return {"coordinates":coordinates,"offset":offset}
    # Warm ties deliberately differ from a fresh index and brute list order.
    ties = [entry([[x,x]]+[[0,0]]*5) for x in [-10,10]]
    yield command(19,ties,[[x]+[0]*5 for x in [10,0,-10,0,10,0]])
    for branching in [2,19,32]:
        yield command(branching,[entry([[0,0]]*6) for _ in range(40)],[[0]*6,[1]*6,[0]*6])
    yield command(19,[],[[0]*6])
    for branching in range(2,33):
        counts = sorted({1,branching-1,branching,branching+1,branching**2-1,branching**2,branching**2+1})
        for count in counts:
            entries = []
            for i in range(count):
                coordinates = []
                for _ in range(6):
                    low = rng.randrange(-20000,20001)
                    coordinates.append([low,low+rng.randrange(0,4001)])
                if i and i%7 == 0:
                    entries.append(dict(entries[i-1]))
                else:
                    entries.append(entry(coordinates,rng.randrange(0,10001)))
            targets = [[rng.randrange(-30000,30001) for _ in range(6)] for _ in range(32)]
            targets += targets[:8]
            yield command(branching,entries,targets)
    for count in [6858,6859,6860,65536]:
        entries = [entry([[0,0]]*6) if count == 65536 else entry([[i%17-8,i%17-8]]*6,i%3) for i in range(count)]
        yield command(19,entries,[[x]*6 for x in [-10,0,10,0]])
    extremes = [-2**63,-2**63+1,-2**31,-1,0,1,2**31-1,2**63-2,2**63-1]
    for count in [1,2,19,20,362]:
        entries = [entry([[rng.choice(extremes),rng.choice(extremes)] for _ in range(6)],rng.choice(extremes)) for _ in range(count)]
        yield command(19,entries,[[rng.choice(extremes) for _ in range(6)] for _ in range(128)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare",action="store_true")
    args = parser.parse_args()
    build = lab.LOCAL/"rust-worldgen-parity"
    build.mkdir(parents=True,exist_ok=True)
    report_path = build/"report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare: lab.prepare()
    lock,jars,libraries = base.verify_artifacts()
    resources = noise_resources(jars["server"])
    if not resources: raise ValueError("pinned noise registry is empty")
    if resources != noise_resources(jars["client"]): raise ValueError("client/server noise registry resources differ")
    densities = registry_resources(jars["server"],"density_function")
    if not densities or densities != registry_resources(jars["client"],"density_function"):
        raise ValueError("client/server density registry resources differ or are empty")
    settings = registry_resources(jars["server"],"noise_settings")
    if not settings or settings != registry_resources(jars["client"],"noise_settings"):
        raise ValueError("client/server noise settings resources differ or are empty")
    tags = tag_resources(jars["server"])
    if not tags or tags != tag_resources(jars["client"]):
        raise ValueError("client/server block tag resources differ or are empty")
    def named(resources, kind):
        prefix = f"data/minecraft/worldgen/{kind}/"
        return {"minecraft:"+name[len(prefix):-5]:text for name,text in resources.items()}
    bundle = build/"registries.json"
    named_settings = named(settings,"noise_settings")
    source = lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWorldgenOracle.java"
    exporter = source.with_name("VanillaBiomeExport.java")
    tag_exporter = source.with_name("VanillaTagExport.java")
    sources = [source,exporter,tag_exporter,Path(__file__),lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",
               *sorted((lab.ROOT/"crates/horizon-core").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world").rglob("*.rs")),
               lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock"]
    tested_sources = base.source_hashes(sources)
    classes = build/"classes"
    classes.mkdir(exist_ok=True)
    dependency_path = os.pathsep.join(map(str,libraries))
    base.run(["javac","--release",str(lock["minecraft"]["java_major"]),"-proc:none","-cp",os.pathsep.join([str(jars["server"]),dependency_path]),"-d",str(classes),str(source),str(exporter),str(tag_exporter)])
    preset_files = {}
    preset_tables = None
    for side,jar in jars.items():
        exported = build/f"biome-presets-{side}.json"
        base.run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeExport",str(exported)])
        tables = json.loads(exported.read_text(encoding="utf-8"))
        if not tables: raise ValueError("built-in biome parameter tables are empty")
        if preset_tables is not None and tables != preset_tables: raise ValueError("client/server built-in biome parameter tables differ")
        preset_tables = tables
        preset_files[side] = exported
    block_elements = None
    tag_export_files = {}
    for side,jar in jars.items():
        exported = build/f"blocks-{side}.json"
        base.run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaTagExport",str(exported)])
        elements = json.loads(exported.read_text(encoding="utf-8"))
        if not elements: raise ValueError("block identifiers are empty")
        if block_elements is not None and elements != block_elements: raise ValueError("client/server block identifiers differ")
        block_elements = elements
        tag_export_files[side] = exported
    bundle.write_text(json.dumps({"tags":tags,"block_elements":block_elements,"noises":named(resources,"noise"),"densities":named(densities,"density_function"),"settings":named_settings,"presets":preset_tables},separators=(",",":")),encoding="utf-8")
    commands = list(corpus(resources))
    commands.extend(climate_corpus())
    commands.extend(index_corpus())
    commands.extend(climate_codec_corpus())
    commands.extend(settings_corpus(settings))
    commands.extend(tag_corpus(tags))
    commands.append(f"tag_registry {bundle}")
    commands.extend(aquifer_corpus())
    commands.extend(noise_aquifer_corpus(bundle,named_settings))
    for seed in [0,1,-1,-2**63,2**63-1,0x123456789abcdef]:
        for legacy in [0,1]:
            for optimized in [0,1]:
                commands.append(f"registry {seed} {legacy} {optimized} {bundle}")
                for name in named_settings:
                    commands.append(f"router {seed} {legacy} {optimized} {bundle} {name}")
    inputs = build/"inputs.txt"
    inputs.write_text("\n".join(commands)+"\n",encoding="utf-8")
    fixture_paths = [bundle,inputs,*preset_files.values(),*tag_export_files.values()]
    tested_fixtures = base.source_hashes(fixture_paths)
    base.run(["cargo","build","--release","--bin","horizon-worldgen-parity"])
    metadata = json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary = Path(metadata["target_directory"])/"release"/("horizon-worldgen-parity.exe" if os.name == "nt" else "horizon-worldgen-parity")
    candidate = build/"rust.txt"
    with inputs.open("rb") as stdin,candidate.open("wb") as stdout: base.run([str(binary)],stdin=stdin,stdout=stdout)
    injected_failures = Counter()
    aquifer_outcomes = Counter()
    with candidate.open(encoding="utf-8") as stream:
        for line in stream:
            if line.startswith("sample failure "): injected_failures[line.strip().split()[-1]] += 1
            elif line.startswith("aquifer result "): aquifer_outcomes[line.strip()[len("aquifer result "):]] += 1
    if dict(injected_failures) != {str(field):3 for field in range(6)}:
        raise ValueError(f"Aquifer failure corpus did not hit every injected failure: {injected_failures}")
    references = []
    for side,jar in jars.items():
        reference = build/f"vanilla-{side}.txt"
        base.run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaWorldgenOracle",str(inputs),str(reference)])
        matched = base.compare(reference,candidate)
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched,"biome_export_sha256":lab.digest(preset_files[side],"sha256"),"block_export_sha256":lab.digest(tag_export_files[side],"sha256")})
        print(f"{side}: {matched:,} world-generation codec output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources)
    base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report = {"schema":1,"minecraft":lock["minecraft"]["version"],"platform":platform.platform(),
              "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
              "manifest_sha1":lock["minecraft"]["manifest_sha1"],"commands":len(commands),"command_counts":dict(Counter(line.split()[0] for line in commands)),"noise_aquifer_injected_failures":dict(injected_failures),"noise_aquifer_outcomes":dict(aquifer_outcomes),"block_tag_entries":len(tags),"block_identifier_entries":len(block_elements),"block_tag_resources_sha256":{name:hashlib.sha256(text.encode("utf-8")).hexdigest() for name,text in tags.items()},"noise_registry_entries":len(resources),"density_registry_entries":len(densities),"noise_settings_entries":len(settings),"biome_parameter_entries":{name:len(json.loads(text)) for name,text in preset_tables.items()},
              "biome_tables_sha256":{name:hashlib.sha256(text.encode("utf-8")).hexdigest() for name,text in preset_tables.items()},"fixtures_sha256":tested_fixtures,
              "noise_resources_sha256":{name:hashlib.sha256(text.encode("utf-8")).hexdigest() for name,text in resources.items()},
              "density_resources_sha256":{name:hashlib.sha256(text.encode("utf-8")).hexdigest() for name,text in densities.items()},
              "noise_settings_resources_sha256":{name:hashlib.sha256(text.encode("utf-8")).hexdigest() for name,text in settings.items()},
              "inputs_sha256":lab.digest(inputs,"sha256"),"rust_output_sha256":lab.digest(candidate,"sha256"),"sources_sha256":tested_sources,"references":references,
              "scope":"Direct normal-noise JSON decoding using the actual pinned vanilla CODEC. Every bundled noise registry resource, explicit/default parameters, empty and full amplitude modifiers, normalization modes, inclusive field bounds with signed-zero comparator ordering, malformed types, null fields, duplicate and ignored keys, decimal/exponent integer truncation and wrapping, signed zero, seeded modern/legacy and Nether construction, octave metadata, range and normalization bits, scalar noise values and parent random state. JSON resource contents stay private. Strict JSON parsing; no claim for Gson's permissive non-JSON syntax. Bundled density registries are also decoded with actual vanilla registry-aware CODECs: named forward references, all bundled function kinds, declared range/axes, raw compilation errors, seeded optimized roots, cache allocation, scalar values, strided bulk values, repeated cache hits and surface-volume errors are compared using RandomState's actual compile context. All seven bundled noise-setting resources contribute their actual eight-field NoiseRouter.CODEC objects to RandomState. Router fields are compiled in declaration order with a shared compiler/context; cache allocation, raw/optimized scalar and repeated strided volume values, successful single-layer surface volumes and invalid surface-volume shapes are compared. Actual RandomState climate sampler binding and quart-to-block coordinate wrapping are checked at ordinary and extreme positions. Float-to-long climate quantization and long-to-float conversion include subnormals, signed zeros, NaNs, infinities, cast saturation and seeded random bit patterns. Actual vanilla Climate.Parameter and ParameterPoint JSON codecs compare scalar, two-element list and min/max object intervals, float endpoint bounds/ordering, required coordinate/offset fields, negative zero, precision-sensitive float parsing, malformed types and duplicate keys. Climate parameter distances, wrapping squared fitness, stable search-tree leaf order and nearest lookup are compared across every supported branching value 2..32, sizes around branch powers, the 65536-entry API bound, duplicates, warm ties, fresh lookups, brute-force lookup and extreme signed-long endpoints/targets. The Rust tree is immutable with per-worker search state instead of hidden thread locals; invalid or degenerate searches return typed errors. Inline arbitrary distance metrics and behavior outside the bounded API are not covered. Built-in Overworld and Nether quantized biome parameter tables are exported independently by each authenticated JAR and required to agree before Rust consumes them. The Rust multi-noise resolver and actual vanilla MultiNoiseBiomeSource compare biome identities through scalar climate queries and prepared bulk quart volumes, including forward/reverse/repeated query order, wrapped extreme origins and shared previous-result state. Biome holder keys in the reference are registration placeholders; no biome property or gameplay behavior is evaluated. The Rust loader reads a Horizon quantized table format, not the vanilla biome codec, and the preset builders themselves have not been ported. Source and private fixture mutations during validation invalidate the report. The two-field 26.3 NoiseSettings codec, create helper and LevelHeightAccessor clamping are also compared across bundled noise members, dimension/alignment bounds, zero/negative runtime heights, disjoint ranges, wrapped integer arithmetic and decimal/exponent integer conversion. Global fluid cutoff selection and the actual disabled Aquifer implementation are compared with original default block-state identities, extreme sea levels/coordinates, signed-zero/subnormal/nonfinite density and counted picker calls; developer fluid-disable behavior is outside the regular-mode corpus. The actual noise-based Aquifer.Config codec and runtime are compared across bundled aquifer settings, modern/legacy world-seeded aquifer factories, synthetic density roots, nondefault water/lava states, cave/void air, nonfinite sampler overrides, strided volume extents, wrapped coordinates, repeated and reversed queries, counted picker calls, sampler call order/values, injected failures and retry state, scheduling flags, populated location/status caches and sorted surface-cache entries. Oversized or wrapped-invalid grid allocations and developer fluid-disable behavior are outside the bounded regular-mode API. Terrain filling is not yet validated. Router, vertical noise and optional aquifer members are decoded from each noise-setting resource; complete NoiseGeneratorSettings, material rules, nonempty world fields, biome registry/codecs and complete chunk biome assignment, generated chunks, gameplay and rendering remain pending. TagEntry and TagFile codecs, ordered resource-stack replacement, first-occurrence member deduplication, required/optional dependency cycle pruning, lookup order and flags, missing-reference partial results and all bundled block tag members are also compared with actual vanilla TagLoader. Block identifier tables are exported independently by authenticated client/server JARs and required to agree. The bounded tag API supports at most 8192 tags, dependency depth 256 and Java hash buckets without tree nodes; hash-tree ordering and pathological self-cycle failures remain pending. No heightmap or complete terrain integration follows from tag resolution. Inline noise holders and general codec behavior beyond the covered fixtures remain pending."}
    lab.write_json(report_path,report)
    print(f"Scoped world-generation codec parity passed: {report_path}")


if __name__ == "__main__":
    main()
