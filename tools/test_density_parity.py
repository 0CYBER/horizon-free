#!/usr/bin/env python3
"""Compare 26.3 compiled density samplers with checksum-pinned vanilla JARs."""
import argparse
from collections import Counter
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import subprocess

try:
    from . import lab, test_rust_parity
except ImportError:
    import lab
    import test_rust_parity


def number(value):
    return struct.pack(">d", value).hex()


def corpus():
    rng = random.Random(0x484f52495a4f4e44454e53495459)
    edges = [-2147483648, -2147483647, -65536, -64, -1, 0, 1, 64, 65536, 2147483646, 2147483647]
    floats = ["00000000", "80000000", "00000001", "80000001", "3f000000", "bf000000", "3f800000", "bf800000", "7f7fffff", "ff7fffff", "7f800000", "ff800000", "7fc00000", "7fa00001"]
    def probes(index, points=32, exact=False):
        for value in edges:
            yield f"query {index} {value} {value} {value}"
        for _ in range(points):
            position = [rng.choice(edges) if rng.randrange(3) == 0 else rng.randrange(-2**31, 2**31) for _ in range(3)]
            yield f"query {index} " + " ".join(map(str, position))
        for iteration in range(5):
            sizes = [rng.randrange(1, 5), rng.randrange(1, 9), rng.randrange(1, 5)]
            minimum = [rng.choice(edges) for _ in range(3)]
            steps = [rng.choice([1, 4, 16, 65536, 2147483647]) for _ in range(3)]
            yield f"volume {index} " + " ".join(map(str, sizes+minimum+steps)) + f" {0 if exact else iteration % 3} {rng.randrange(-2**63, 2**63)} 2"
    index = 0
    good_children = []
    all_children = []
    for value in floats + [f"{rng.getrandbits(32):08x}" for _ in range(64)]:
        yield f"constant {value}"
        yield from probes(index)
        all_children.append(index)
        good_children.append(index)
        index += 1
    for scenario in range(1800):
        axis, tiling = scenario % 3, (scenario // 3) % 3
        if scenario % 4 == 0:
            start, end = rng.choice(edges), rng.choice(edges)
        elif scenario % 4 == 1:
            start, end = rng.randrange(-2**31, 2**31), rng.randrange(-2**31, 2**31)
        else:
            start, end = rng.randrange(-128, 128), rng.randrange(-128, 128)
        if scenario % 37 == 0:
            end = start
        if scenario % 41 == 0:
            start, end = 0, -1  # MIN / -1 overflow in repeating paths.
        low, high = (rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(2))
        yield f"gradient {axis} {tiling} {start} {end} {low} {high}"
        for coordinate in [start, end, max(-2**31, start-1), min(2**31-1, end+1)]:
            yield f"query {index} {coordinate} {coordinate} {coordinate}"
        yield from probes(index)
        all_children.append(index)
        if tiling == 0 or start != end:
            good_children.append(index)
        index += 1
    scales = [0.0, -0.0, .001, -.001, .125, .5, 1.0, 4.0, 1e100, float("inf"), float("-inf"), float("nan")]
    noise_children = []
    for scenario in range(240):
        kind = ["perlin", "simplex", "smeared"][scenario % 3]
        stream = ["legacy", "xoroshiro"][(scenario // 3) % 2]
        scale = rng.choice(scales)
        xz, y = (rng.choice(scales) if scenario % 3 == 0 else rng.uniform(-2, 2) for _ in range(2))
        noise_children.append(index)
        yield f"noise {kind} {stream} {rng.randrange(-2**63, 2**63)} {number(scale)} {number(xz)} {number(y)}"
        yield from probes(index)
        good_children.append(index)
        all_children.append(index)
        index += 1
    nested = []
    for scenario in range(1200):
        choices = all_children if scenario % 9 == 0 else good_children
        choices = choices + [i for i, depth in nested if depth < 4]
        x, y, z = (rng.choice(choices) for _ in range(3))
        if scenario % 2 == 0:
            y = -1
        yield f"shift {scenario % 240} {x} {y} {z}"
        yield from probes(index)
        depth = 1 + max([d for i, d in nested if i in [x, y, z]], default=0)
        nested.append((index, depth))
        index += 1
    unary_kinds = ["abs", "negate", "square", "cube", "sqrt", "reciprocal", "sign", "squeeze", "leaky", "add", "sub", "mul", "div", "min", "max", "clamp"]
    arithmetic = []
    for scenario in range(2400):
        kind = unary_kinds[scenario % len(unary_kinds)]
        choices = all_children + [i for i, depth in nested if depth <= 2] + [i for i, depth in arithmetic if depth < 4]
        child = rng.choice(choices)
        a, b = (rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(2))
        yield f"unary {kind} {child} {a} {b}"
        yield from probes(index)
        depth = 1 + max([d for i, d in arithmetic if i == child], default=0)
        arithmetic.append((index, depth))
        index += 1
    binary = []
    for scenario in range(2400):
        kind = ["add", "sub", "mul", "div", "min", "max"][scenario % 6]
        choices = all_children + [i for i, depth in arithmetic if depth <= 2] + [i for i, depth in binary if depth < 4]
        left, right = (rng.choice(choices) for _ in range(2))
        if scenario % 7 == 0:
            left = scenario % len(floats)  # Force scalar shortcuts and exceptional values.
        bound = rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}"
        yield f"binary {kind} {left} {right} {bound}"
        yield from probes(index, exact=True)
        depth = 1 + max([d for i, d in binary if i in [left, right]], default=0)
        binary.append((index, depth))
        index += 1

    selections = []
    for scenario in range(2400):
        choices = all_children + [i for i, depth in arithmetic+binary+selections if depth < 3]
        selected = rng.choice(choices) if scenario % 3 else scenario % len(floats)
        if scenario % 2 == 0:
            mode = (scenario // 2) % 2
            a, b = (rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(2))
            inside, outside = (rng.choice(floats) if mode else str(rng.choice(choices)) for _ in range(2))
            yield f"choice {mode} {selected} {a} {b} {inside} {outside}"
            children = [selected] + ([] if mode else [int(inside), int(outside)])
        else:
            mode = (scenario // 2) % 3
            first = rng.choice(floats) if mode == 1 else str(rng.choice(choices))
            second = rng.choice(floats) if mode == 2 else str(rng.choice(choices))
            yield f"lerp {mode} {selected} {first} {second}"
            children = [selected] + ([] if mode == 1 else [int(first)]) + ([] if mode == 2 else [int(second)])
        yield from probes(index, exact=True)
        depth = 1 + max([d for i, d in arithmetic+binary+selections if i in children], default=0)
        selections.append((index, depth))
        index += 1
    # Known failure nodes make skipped branches, bulk evaluation order and
    # partial writes directly observable at scalar branch endpoints.
    failure = index
    yield "gradient 0 1 0 0 00000000 3f800000"
    index += 1
    for zero in [0, 1, 6, 12]:
        for operation in [f"binary mul {zero} {failure} 00000000", f"binary div {zero} {failure} 00000000",
                          f"choice 0 {zero} 00000000 3f800000 6 {failure}", f"choice 0 {zero} 00000000 3f800000 {failure} 6",
                          f"lerp 0 {zero} 6 {failure}", f"lerp 0 {zero} {failure} 6",
                          f"lerp 1 {zero} 80000000 {failure}", f"lerp 2 {zero} {failure} 80000000"]:
            yield operation
            yield from probes(index, points=1, exact=True)
            index += 1
    for source in range(14):
        for minimum, maximum in [("00000000", "3f800000"), ("80000000", "00000000"), ("7fc00000", "3f800000")]:
            yield f"choice 1 {source} {minimum} {maximum} 80000000 3f800000"
            yield from probes(index, points=1)  # Constant-choice logical tails.
            index += 1

    interval_nodes = []
    for scenario in range(1000):
        choices = all_children + [i for i, depth in arithmetic+binary+selections+interval_nodes if depth < 3]
        source = rng.choice(choices) if scenario % 3 else scenario % len(floats)
        mode = scenario % 2
        count = 1 if mode else rng.choice([0, 1, 2, 3, 4, 8, 16])
        thresholds = [rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(count)]
        branches = [rng.choice(choices) for _ in range(count+1)]
        if scenario % 13 == 0:
            branches[scenario % len(branches)] = failure
        yield f"interval {mode} {source} {count} " + " ".join(thresholds + list(map(str, branches)))
        yield from probes(index, exact=True)
        depth = 1 + max([d for i, d in arithmetic+binary+selections+interval_nodes if i in [source]+branches], default=0)
        interval_nodes.append((index, depth))
        index += 1

    # Range propagation is independently checked on bit-pattern endpoints.
    for a in floats:
        for b in floats:
            for mode in range(6):
                yield f"bounds {mode} {a} {b} {a} {b} {a} {b}"
    for scenario in range(24000):
        values = [rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(6)]
        yield f"bounds {scenario % 6} " + " ".join(values)

    for scenario in range(32):
        stream = ["legacy", "xoroshiro"][scenario % 2]
        yield f"noise normal {stream} {rng.randrange(-2**63,2**63)} {number(0)} {number(.25)} {number(.125)}"
        yield from probes(index)
        index += 1
    for noise_id in range(272):
        for kind in range(3):
            yield f"offset {kind} {noise_id}"
            yield from probes(index)
            index += 1
    for scenario in range(192):
        stream = ["legacy", "xoroshiro"][scenario % 2]
        # The first cases include the actual overworld/nether and End parameters.
        if scenario < 4:
            parameters = [.25 if scenario < 2 else 1.0, .125 if scenario < 2 else 1.0, 80.0, 160.0, 8.0 if scenario < 2 else 4.0]
        elif scenario % 3 == 0:
            parameters = [rng.choice([0.0, -0.0, .001, .25, 1.0, -1.0, 1000.0, 1e100, float("inf")]), rng.choice([0.0, -0.0, .001, .125, 1.0, -1.0, 1000.0]), rng.choice([0.0, -0.0, 80.0, -80.0, 1e100, float("inf")]), rng.choice([160.0, -160.0, 1e100, float("inf")]), rng.choice([0.0, -0.0, 1.0, 4.0, 8.0, -8.0, 1e100])]
        else:
            parameters = [rng.uniform(.001,1000), rng.uniform(.001,1000), rng.uniform(.001,1000), rng.uniform(.001,1000), rng.uniform(1,8)]
        yield f"blended {stream} {rng.randrange(-2**63,2**63)} " + " ".join(map(number,parameters))
        yield from probes(index, points=96, exact=True)
        yield f"volume {index} 3 9 3 -32 -64 16 4 8 4 0 {scenario} 2"
        index += 1
    for scenario in range(320):
        point = [rng.choice(edges) if scenario % 2 == 0 else rng.randrange(-2**31,2**31) for _ in range(3)]
        yield f"distance {scenario % 4} " + " ".join(map(str,point))
        yield f"query {index} " + " ".join(map(str,point))
        yield from probes(index)
        index += 1
    for scenario in range(12000):
        values = [rng.choice(floats) if scenario % 3 == 0 else f"{rng.getrandbits(32):08x}" for _ in range(3)]
        yield "metric " + " ".join(values)
    for stream in ["legacy", "xoroshiro"]:
        for parameters in [[1,float("nan"),80,160,8], [1,1,80,float("nan"),8], [1,0,80,0,8], [1,1,80,160,float("nan")], [1,0,80,160,float("inf")]]:
            yield f"blendfail {stream} -1 " + " ".join(map(number,parameters))
    for scenario in range(256):
        stream = ["legacy", "xoroshiro"][scenario % 2]
        octave = rng.choice([0,-1,-7,-15,-31,-63,-127,1])
        smear = rng.choice([0.0,-0.0,.125,1.0,-1.0,1000.0,1e100,float("inf"),float("nan")])
        amplitude = rng.choice([0.0,-0.0,1.0,-1.0,.9999847412109375,12.75,1e100,float("inf"),float("nan")])
        yield f"blendfbm {stream} {rng.randrange(-2**63,2**63)} {octave} {number(smear)} {number(amplitude)}"

    for scenario in range(192):
        source = rng.choice(noise_children) if scenario % 4 == 1 else (rng.choice(all_children) if scenario % 4 else scenario % len(floats))
        yield f"counted {source}"
        counted = index
        index += 1
        # Some nested caches share a node; IDs remain distinct on every path.
        if scenario % 3 == 0:
            yield f"cache {scenario + 256} {counted}"
            counted = index
            index += 1
        yield f"cache {scenario} {counted}"
        cached = index
        index += 1
        for enabled in [0, 1]:
            yield f"context {enabled}"
            for point in [[0,0,0], [0,0,0], [2**26,4096,-2**26], [-17,-64,32], [-17,-64,32]]:
                yield f"query {cached} " + " ".join(map(str,point))
                yield f"counts {scenario}"
            for minimum in [[-17,-64,32], [-17,-64,32], [2147483647,-2147483648,0], [-17,-64,32]]:
                yield f"volume {cached} 2 3 2 " + " ".join(map(str,minimum)) + f" 4 1 4 0 {scenario} 2"
                yield f"counts {scenario}"
                for offset in [[0,0,0],[0,0,0],[4,1,4],[1,0,0],[0,3,0]]:
                    point = [((a+b+2**31) % 2**32)-2**31 for a,b in zip(minimum,offset)]
                    yield f"query {cached} " + " ".join(map(str,point))
                    yield f"counts {scenario}"
    for scenario in range(10000):
        sizes = [rng.randrange(1,9) for _ in range(3)]
        minimum = [rng.choice(edges) for _ in range(3)]
        steps = [rng.choice([1,4,16,65536,2147483647]) for _ in range(3)]
        offsets = [rng.randrange(-1, sizes[i]+1)*steps[i] + (1 if scenario % 5 == 0 else 0) for i in range(3)]
        point = [((a+b+2**31) % 2**32)-2**31 for a,b in zip(minimum,offsets)]
        yield "lookup " + " ".join(map(str,sizes+minimum+steps+point))

    sliced = []
    for scenario in range(384):
        mode = (scenario // 8) % 4
        a, b = rng.choice(edges), rng.choice(edges)
        # Force buffer tails on primitive inputs, and nested/cached graphs on
        # exact buffers. IDs are never reused for different cached functions.
        tails = scenario % 8 == 0
        choices = all_children + [i for i, depth in binary if depth < 3] + [i for i, depth in sliced if depth < 3]
        source = scenario % len(floats) if tails else rng.choice(choices)
        depth = 1 + max([d for i, d in sliced if i == source], default=0)
        yield f"counted {source}"
        child = index
        counter = 192 + scenario
        index += 1
        if scenario % 8 == 1:
            yield f"cache {1024 + scenario} {child}"
            child = index
            index += 1
        yield f"slice {mode} {child} {a} {b}"
        sampler = index
        sliced.append((sampler, depth))
        index += 1
        for enabled in [0, 1]:
            yield f"context {enabled}"
            for command in probes(sampler, points=8, exact=not tails):
                yield command
                yield f"counts {counter}"
            for match_minimum in [False, True]:
                for match_size in [False, True]:
                    sizes = [2, 5, 3]
                    minimum = [-17, -64, 32]
                    fixed = [(mode, a)] if mode < 3 else [(0, a), (2, b)]
                    for axis, value in fixed:
                        sizes[axis] = 1 if match_size else sizes[axis]
                        minimum[axis] = value if match_minimum else (value ^ 1)
                    yield f"volume {sampler} " + " ".join(map(str, sizes+minimum+[4,8,16])) + f" {2 if tails else 0} {scenario} 2"
                    yield f"counts {counter}"

    for scenario in range(384):
        xz = [1, 2, 3, 4, 7, 8, 16, 31][scenario % 8]
        cell_y = [1, 2, 3, 4, 7, 8, 16, 31][(scenario // 8) % 8]
        source = rng.choice(all_children + [i for i, d in binary if d < 3] + [i for i, d in sliced if d < 3])
        if scenario % 3 == 0:
            source = rng.choice(noise_children)
        elif scenario % 3 == 1:
            source = (scenario // 3) % len(floats)
        yield f"counted {source}"
        counter = 576 + scenario
        child = index
        index += 1
        if scenario % 5 == 0:
            yield f"cache {2048 + scenario} {child}"
            child = index
            index += 1
        yield f"interpolate {child} {xz} {cell_y}"
        sampler = index
        index += 1
        for enabled in [0, 1]:
            yield f"context {enabled}"
            for value in edges + [rng.randrange(-2**31, 2**31) for _ in range(8)]:
                yield f"query {sampler} {value} {value} {value}"
                yield f"counts {counter}"
            for point in [[0,0,0],[-xz,-cell_y,xz],[xz,cell_y,-xz],[-1,-1,-1]]:
                yield f"query {sampler} " + " ".join(map(str,point))
                yield f"counts {counter}"
            for iteration in range(12):
                sizes = [rng.randrange(1,6),rng.randrange(1,17),rng.randrange(1,6)]
                minimum = [rng.choice([-2147479552,-65,-64,-33,-17,-1,0,1,16,32,2147479551]) for _ in range(3)]
                steps = [1,1,1] if iteration < 4 else ([xz,cell_y,xz] if iteration < 8 else [2,3,4])
                if iteration in [4,5]:
                    minimum = [-xz,-cell_y,xz]
                if iteration == 6:
                    sizes = [1,1,1]
                    minimum = [0,0,0]
                    steps = [1,1,1]
                if iteration == 7:
                    sizes = [2,1,3]
                    minimum = [0,0,0]
                    steps = [xz,1,xz]
                yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" {2 if scenario % 3 == 1 and scenario % 5 != 0 else 0} {scenario} 2"
                yield f"counts {counter}"

    for scenario in range(384):
        lower = [-128,-64,-63,0,64][scenario % 5]
        height = [1,2,3,4,8,16,31][scenario % 7]
        density = rng.choice(all_children)
        if scenario % 2 == 0:
            upper_bits = (floats + [struct.pack(">f",v).hex() for v in [-128,-64,-1,1,64,256,384,100.5]])[(scenario // 2) % 22]
            upper_value = struct.unpack(">f",bytes.fromhex(upper_bits))[0]
            if not (-512 <= upper_value <= 512):
                density = 6  # Positive density makes extreme-height searches terminate immediately.
                if scenario % 4 == 0:
                    height = 2147483647
            yield f"constant {upper_bits}"
        else:
            end = -64 if scenario % 17 == 0 else 64
            yield f"gradient {(scenario // 2) % 3} {(scenario // 6) % 3} -64 {end} c3000000 43800000"
        upper = index
        index += 1
        yield f"counted {density}"
        density = index
        index += 1
        yield f"counted {upper}"
        upper = index
        index += 1
        if scenario % 7 == 0:
            yield f"cache {4096 + scenario} {density}"
            density = index
            index += 1
        mode = (scenario // 2) % 2
        yield f"surface {mode} {density} {upper} {lower} {height}"
        sampler = index
        index += 1
        for enabled in [0,1]:
            yield f"context {enabled}"
            for value in edges + [-129,-65,-63,63,127,128,255,256]:
                yield f"query {sampler} {value} {value} {value}"
                yield f"counts {960 + scenario * 2}"
                yield f"counts {961 + scenario * 2}"
            for iteration in range(6):
                sizes = [rng.randrange(1,5),1 if iteration < 4 else 3,rng.randrange(1,5)]
                minimum = [rng.choice(edges),0 if iteration % 2 else -64,rng.choice(edges)]
                steps = [rng.choice([1,4,65536,2147483647]),8,rng.choice([1,4,65536,2147483647])]
                yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" {iteration % 3} {scenario} 2"
                yield f"counts {960 + scenario * 2}"
                yield f"counts {961 + scenario * 2}"

    yield "context 0"
    spline_index = 0
    for value in floats + [f"{rng.getrandbits(32):08x}" for _ in range(64)]:
        yield f"spconstant {value}"
        if math.isnan(struct.unpack(">f",bytes.fromhex(value))[0]):
            yield f"spfail {spline_index}"
        else:
            yield f"spsampler {spline_index}"
            yield from probes(index, points=8)
            index += 1
        spline_index += 1
    for scenario in range(256):
        sources = [rng.choice(all_children),rng.choice(noise_children),rng.choice(all_children)]
        if scenario % 4 in [0,1]:
            sources[0] = 12 if scenario % 4 == 0 else 4
        coordinates = []
        for source in sources:
            yield f"counted {source}"
            coordinates.append(index)
            index += 1
        nodes = []
        for node in range(3):
            coordinate = coordinates[0 if node == 2 or (node == 0 and scenario % 2 == 0) else node+1]
            count = [1,2,3,8][(scenario+node) % 4]
            locations = [float(i)-count*.5 for i in range(count)]
            if scenario % 7 == 0:
                locations = [struct.unpack(">f",bytes.fromhex(rng.choice(floats)))[0] for _ in locations]
            elif scenario % 7 == 1:
                locations = [0.0 for _ in locations]
            fields = []
            for point, location in enumerate(locations):
                derivative = rng.choice(floats) if scenario % 3 == 0 else struct.pack(">f",rng.uniform(-2,2)).hex()
                value = rng.randrange(78) if node < 2 else rng.choice(nodes)
                if node == 2 and point == count-1:
                    value = nodes[0]  # Force repeated NaN coordinate reads in scalar queries.
                fields += [struct.pack(">f",location).hex(),derivative,str(value)]
            yield f"sppoints {coordinate} {count} " + " ".join(fields)
            nodes.append(spline_index)
            spline_index += 1
        yield f"spsampler {nodes[-1]}"
        sampler = index
        index += 1
        for enabled in [0,1]:
            yield f"context {enabled}"
            for command in probes(sampler, points=8, exact=True):
                yield command
                for counter in range(1728+scenario*3,1731+scenario*3):
                    yield f"counts {counter}"

    for spline in range(spline_index):
        for scenario in range(16):
            values = [rng.choice(floats) if scenario % 2 else struct.pack(">f",rng.uniform(-4,4)).hex() for _ in range(6)]
            if scenario == 0:
                values = ["ff800000","7f800000"] * 3
            if scenario == 1:
                values = ["7fc00000"] * 6
            yield f"sprange {spline} " + " ".join(values)

    for scenario in range(128):
        yield "greset"
        for value in ["00000000","80000000","3f800000","bf800000","42000000",rng.choice(floats[:12])]:
            yield f"gconst {value}"
        yield f"ggradient {scenario % 3} {(scenario // 3) % 3} -64 {(-64 if scenario % 17 == 0 else 64)} bf800000 3f800000"
        for slot, source in enumerate([rng.choice(noise_children),rng.choice(all_children)]):
            yield f"counted {source}"
            counted = index
            index += 1
            yield f"gbind {counted} bf800000 3f800000 {(scenario+slot) % 8}"
        yield "gcache 6"
        yield "gcache 6"
        yield "gsame 9 10"
        yield "gcache 9"
        yield f"gslice {0 if scenario % 2 else 2} {-17 if scenario % 3 else 0} 7"
        yield f"gslice {2 if scenario % 2 else 0} {32 if scenario % 3 else 0} 12"
        yield f"gunary {scenario % 9} 9"
        yield f"gbinary {scenario % 6} {5 if scenario % 4 == 0 else 14} {5 if scenario % 4 == 1 else 8}"
        yield "gclamp 15 bf800000 3f800000"
        yield "gchoice 7 bf000000 3f000000 16 14"
        yield f"glerp 16 {2 if scenario % 3 == 0 else 17} {5 if scenario % 3 == 1 else 14}"
        yield "ginterp 18 4 8"
        yield "gsurface 18 4 -64 8"
        yield "gbinary 5 19 20"
        yield "gcache 21"
        yield "gbinary 0 22 11"
        for node in range(24):
            yield f"ginfo {node}"
        yield "gtry 0 9"
        for mode, roots in [(0,[5,6,13]),(1,[5,6,9,11,13,14,15,16,17,18,19,20,21,22,23])]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    for point in [[0,0,0],[-17,-64,32],[1,7,-1],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]:
                        yield f"query {sampler} " + " ".join(map(str,point))
                        for counter in [2496+scenario*2,2497+scenario*2]:
                            yield f"counts {counter}"
                    for sizes, minimum, steps in [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4]),([1,3,1],[0,0,0],[1,1,1])]:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" {2 if root == 5 else 0} {scenario} 2"
                        for counter in [2496+scenario*2,2497+scenario*2]:
                            yield f"counts {counter}"
        yield "gconst 7fc00000"
        yield "gcache 24"
        yield "gcache 25"
        for root in [26,26,25]:
            yield f"gtry 1 {root}"
        yield "gtry 0 25"
        yield "ginfo 24"
        yield "ginfo 25"
        yield "gconst 7fa00001"
        yield "gsame 24 27"
        yield "gconst 80000000"
        yield "gsame 1 28"
        yield "gsame 0 28"
        yield f"gdistance {-2147483648 if scenario % 3 == 0 else -17} {scenario-64} {2147483647 if scenario % 3 == 1 else 32} {scenario % 4}"
        yield f"ginterval 7 1 {floats[scenario % len(floats)]} 8 29"
        thresholds = [[], ["00000000"], ["7fc00000"], ["80000000", "00000000", "7f800000"], ["3f800000", "bf800000", "7fc00000"]][scenario % 5]
        branches = [29, 13, 7, 2][:len(thresholds)+1]
        yield f"ginterval 8 {len(thresholds)} " + " ".join(thresholds + list(map(str,branches)))
        yield "gcache 30"
        yield "ginterval 9 1 00000000 30 29"
        yield "ginterval 24 1 00000000 29 2"
        yield "ginterval 7 1 7fc00000 8 29"
        yield "ginterval 7 1 7fa00001 8 29"
        yield "gsame 35 36"
        yield "ginterval 7 1 00000000 8 29"
        yield "ginterval 7 1 80000000 8 29"
        yield "gsame 37 38"
        yield "gcache 35"
        yield "gcache 36"
        yield "gtry 1 39"
        yield "gtry 1 40"
        yield "gbinary 0 37 37"
        yield "gbinary 0 38 38"
        yield "gsame 41 42"
        yield "gcache 41"
        yield "gcache 42"
        yield "gtry 1 43"
        yield "gtry 1 44"
        for node in range(29,41):
            yield f"ginfo {node}"
        for mode, roots in [(0,[29,30,31,34]),(1,[29,30,31,32,33,34])]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    for point in [[0,0,0],[-17,-64,32],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]:
                        yield f"query {sampler} " + " ".join(map(str,point))
                        for counter in [2496+scenario*2,2497+scenario*2]:
                            yield f"counts {counter}"
                    for sizes, minimum, steps in [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4])]:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" {2 if root == 29 else 0} {scenario} 2"
                        for counter in [2496+scenario*2,2497+scenario*2]:
                            yield f"counts {counter}"
        for value in ["00000000", "80000000", "3f800000", "bf800000", "7fc00000", "7f800000"]:
            yield f"gsconstant {value}"
        for spline in [0,1,2,4,5]:
            yield f"gspline {spline}"
        # Shared and structurally equal coordinates, including zero-hash variants.
        coordinate = [7,8,13][scenario % 3]
        point_fields = []
        for point, location in enumerate(["bf800000", "00000000", "3f800000"]):
            derivative = rng.choice(floats) if scenario % 4 == 0 else struct.pack(">f",rng.uniform(-2,2)).hex()
            value = rng.randrange(6) if scenario % 4 == 0 else point % 4
            point_fields += [location, derivative, str(value)]
        yield f"gspoints {coordinate} 3 " + " ".join(point_fields)
        yield f"gspoints {[7,8,13][(scenario+1)%3]} 2 bf000000 00000000 6 3f000000 00000000 6"
        yield f"gspoints {9 if scenario % 2 else 29} 3 bf800000 00000000 7 00000000 3f000000 6 3f800000 00000000 7"
        for spline in [6,7,8,8]:
            yield f"gspline {spline}"
        yield "gsame 52 53"
        yield f"gspoints {coordinate} 3 " + " ".join(point_fields)
        yield "gspline 9"
        yield "gsame 50 54"
        for coordinate in [37,38]:
            yield f"gspoints {coordinate} 2 bf800000 00000000 0 3f800000 00000000 2"
        yield "gspline 10"
        yield "gspline 11"
        yield "gsame 55 56"
        yield "gcache 55"
        yield "gcache 56"
        yield "gtry 1 57"
        yield "gtry 1 58"
        yield "gcache 52"
        yield "gtry 1 59"
        yield "gtry 1 59"
        yield "gspoints 24 1 00000000 00000000 0"
        yield "gspline 12"
        yield "gspoints 48 1 00000000 00000000 0"
        yield "gspline 13"
        yield f"gbind {counted} 7fc00000 7fc00000 2"
        yield "gspoints 62 1 00000000 00000000 12"
        yield "gspline 14"
        for node in range(45,64):
            yield f"ginfo {node}"
        for mode in [0,1]:
            for root in [48,60,61,63]:
                yield f"gtry {mode} {root}"
        for mode, roots in [(0,[45,46,47,49,50,54,55,56]),(1,[45,46,47,49,50,51,52,53,54,55,56,57,58])]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    for point in [[0,0,0],[-17,-64,32],[1,7,-1],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]:
                        yield f"query {sampler} " + " ".join(map(str,point))
                        for counter in [2496+scenario*2,2497+scenario*2]:
                            yield f"counts {counter}"
                    for sizes, minimum, steps in [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4]),([1,3,1],[0,0,0],[1,1,1])]:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" {2 if root in [45,46,47,49] else 0} {scenario} 2"
                        for counter in [2496+scenario*2,2497+scenario*2]:
                            yield f"counts {counter}"

    for scenario in range(64):
        seed = [0, 1, -1, -9223372036854775808, 9223372036854775807][scenario % 5] if scenario < 10 else rng.randrange(-2**63,2**63)
        yield f"gseed {seed} {scenario % 2}"
        definitions = [
            ("horizon:primary", -7, 1.0, 4, scenario % 3, [1.0,0.0,-0.5,1.0] if scenario % 4 == 0 else []),
            ("minecraft:nether/temperature", -7, 1.0, 4, 2, []),
            ("minecraft:nether/vegetation", -8, 1.0, 4, 2, [1.0,0.0,0.5,1.0]),
            ("horizon:empty", 0, 1.0, 0, 1, []),
            ("horizon:secondary", -3, 0.75, 3, 0, [1.0,-0.5,0.0]),
        ]
        for name, base, amplitude, count, normalization, modifiers in definitions:
            yield f"gregister {name} {base} {number(amplitude)} {count} {normalization} {len(modifiers)} " + " ".join(number(v) for v in modifiers)
        yield "gready"
        for noise in range(5):
            yield f"ginstance {noise}"
        for value in ["00000000", "80000000", "3f800000"]:
            yield f"gconst {value}"
        yield "ggradient 1 0 -64 320 bf800000 3f800000"
        yield "ggradient 0 0 -32 32 bf800000 3f800000"
        for kind in range(3):
            yield f"goffset 0 {kind}"
        yield "gcache 6"
        yield f"gnoise 0 {number(.125)} {number(.25)} 0 0 0"
        yield f"gnoise 0 {number(.125)} {number(.25)} 6 0 7"
        yield f"gnoise 0 {number(.125)} {number(.25)} 6 1 7"
        yield f"gnoise 0 {number(.125)} {number(.25)} 6 3 7"
        yield f"gnoise 3 {number(1)} {number(0)} 0 0 0"
        yield f"gnoise 1 {number(.25)} {number(0)} 0 0 0"
        yield f"gnoise 2 {number(.25)} {number(0)} 0 0 0"
        yield f"gnoise 4 {number(-0.0)} {number(0)} 0 0 0"
        scale = [0.0, -0.0, .125, 1.0, float('inf'), float('nan'), -.5][scenario % 7]
        yield f"gnoise 4 {number(scale)} {number(scale)} 4 3 2"
        yield f"gnoise 0 {number(.125)} {number(.25)} 8 0 7"
        yield "gcache 12"
        parameters = [(.25,.125,80.,160.,8.), (.25,.375,80.,60.,8.), (.25,.25,80.,160.,4.)][scenario % 3]
        yield "gblended " + " ".join(number(v) for v in parameters)
        yield "gbinary 0 19 20"
        yield "gsconstant 00000000"
        yield "gsconstant 3f800000"
        yield "gspoints 9 2 bf800000 00000000 0 3f800000 00000000 1"
        yield "gspoints 12 2 bf800000 00000000 2 3f800000 00000000 2"
        yield "gspline 3"
        yield "ginterp 21 4 8"
        yield "gcache 23"
        yield "gclamp 22 bf800000 3f800000"
        yield f"gnoise 0 {number(.125)} {number(.25)} 0 0 0"
        yield "gsame 9 26"
        yield f"gnoise 0 {number(.125)} {number(.25)} 1 0 0"
        yield "goffset 3 1"
        yield f"gnoise 4 {number(0)} {number(0)} 0 0 0"
        yield "gend"
        yield "gend"
        yield "gsame 30 31"
        yield "gcache 30"
        yield "gbinary 0 30 12"
        yield "gclamp 33 bf800000 3f800000"
        yield "ginterp 33 4 8"
        yield "gspoints 30 2 bf800000 00000000 0 3f800000 00000000 1"
        yield "gspline 4"
        yield "gsurface 33 2 -64 8"
        for node in range(38):
            yield f"ginfo {node}"
        yield "gtry 0 8"
        for mode, roots in [(0,[5,6,7,9,10,11,12,13,14,15,16,17,20,22,25,27,28,29,30,31,33,34,35,36,37]),(1,[5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37])]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    points = [[0,0,0],[-17,-64,32],[1,7,-1],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]
                    if root >= 30:
                        points += [[1024,80,1024],[-1025,-64,1024],[2048,256,-4096]]
                    for point in points:
                        yield f"query {sampler} " + " ".join(map(str,point))
                    volumes = [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4]),([1,3,1],[0,0,0],[1,1,1])]
                    if root >= 30:
                        volumes.append(([3,2,3],[1024,-64,1024],[16,64,16]))
                    for sizes, minimum, steps in volumes:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" 0 {scenario} 2"
        for noise in range(5):
            yield f"ginstance {noise}"

    for scenario in range(6):
        yield f"gseed {-9223372036854775808 if scenario % 2 else 9223372036854775807} {scenario % 2}"
        base, count = [(0,3), (1,1), (-1,8)][scenario // 2]
        yield f"gregister minecraft:nether/temperature {base} {number(1)} {count} 1 0"
        yield "gready"
        yield "gconst 00000000"
        yield f"gnoise 0 {number(.25)} {number(0)} 0 0 0"
        yield "goffset 0 1"
        yield "gcache 1"
        for root in [1,2,3]:
            yield f"ginfo {root}"
            for mode in [0,1,1]:
                yield f"gtry {mode} {root}"

    # Rounding is appended so the established corpus keeps its original inputs.
    rounding_values = floats + ["3effffff","3f000001","beffffff","bf000001",
        "3fbfffff","3fc00000","3fc00001","bfbfffff","bfc00000","bfc00001",
        "4affffff","4b000000","4effffff","4f000000","ceffffff","cf000000"]
    rounding_inputs = []
    for value in rounding_values:
        yield f"constant {value}"
        rounding_inputs.append(index)
        index += 1
    multiples = [-1] + [rounding_inputs[i] for i in [0,1,2,4,5,6,7,10,11,12]]
    for kind in range(4):
        for input in rounding_inputs:
            for multiple in multiples:
                yield f"round {kind} {input} {multiple}"
                yield f"query {index} -17 -64 32"
                yield f"volume {index} 2 3 2 -17 -64 32 4 8 4 {2 if multiple < 0 else 0} {index} 2"
                index += 1
    for scenario in range(256):
        input = rng.choice(all_children)
        multiple = -1 if scenario % 5 == 0 else rng.choice(all_children)
        yield f"round {scenario % 4} {input} {multiple}"
        yield from probes(index, points=8, exact=multiple >= 0)
        index += 1

    for scenario in range(64):
        yield "greset"
        for value in ["00000000","80000000","3f800000","bf800000","7fc00000","7f800000",rounding_values[scenario % len(rounding_values)]]:
            yield f"gconst {value}"
        yield "ggradient 1 0 -64 64 bf800000 3f800000"
        for slot, source in enumerate([rng.choice(noise_children),rng.choice(all_children)]):
            yield f"counted {source}"
            counted = index
            index += 1
            yield f"gbind {counted} bf800000 3f800000 {7 if slot == 0 else scenario % 8}"
        yield "gcache 8"
        for input, multiple, offset in [(8,2,0),(8,9,1),(7,0,2),(7,1,3),(9,6,0),(6,3,1),(10,9,2)]:
            yield f"ground {(scenario+offset) % 4} {input} {multiple}"
        yield "gcache 12"
        yield f"ground {(scenario+3) % 4} 18 11"
        yield f"ground {scenario % 4} 6 4"
        yield f"ground {scenario % 4} 6 5"
        yield f"gbind {rounding_inputs[6]} 3f800000 3f800000 0"
        yield f"ground {scenario % 4} 9 22"
        for node in range(24):
            yield f"ginfo {node}"
        for mode, roots in [(0,[11,12,13,14,15,16,20,21,23]),(1,list(range(11,22))+[23])]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    for point in [[0,0,0],[-17,-64,32],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]:
                        yield f"query {sampler} " + " ".join(map(str,point))
                        for counter in [2752+scenario*2,2753+scenario*2]:
                            yield f"counts {counter}"
                    for sizes, minimum, steps in [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4])]:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" 0 {scenario} 2"
                        for counter in [2752+scenario*2,2753+scenario*2]:
                            yield f"counts {counter}"
        yield "gcache 6"
        yield f"ground {scenario % 4} 24 2"
        for root in [24,25]:
            for mode in [0,1,1]:
                yield f"gtry {mode} {root}"

    for scenario in range(768):
        child = rounding_inputs[scenario % len(rounding_inputs)] if scenario < 128 else rng.choice(all_children)
        yield f"unary log {child} 00000000 00000000"
        yield from probes(index, points=8)
        index += 1

    power_values = floats + ["40000000","c0000000","40400000","c0400000","3fc00000","bfc00000"]
    for base in power_values:
        for first in power_values:
            for last in power_values:
                yield f"bounds 2 {base} {base} {first} {last} 00000000 3f800000"
                yield f"bounds 3 {first} {last} {base} {base} 00000000 3f800000"

    for scenario in range(64):
        yield "greset"
        for value in ["00000000","80000000","3f800000","bf800000","7fc00000","7f800000",rounding_values[scenario % len(rounding_values)]]:
            yield f"gconst {value}"
        yield "ggradient 1 0 -64 64 00000000 40800000"
        yield "ggradient 0 0 -32 32 bf800000 3f800000"
        yield f"counted {rng.choice(all_children)}"
        counted = index
        index += 1
        low, high = [("00000000","40800000"),("bf800000","3f800000"),("ff800000","7f800000"),("80000000","80000000"),("bf800000","bf000000"),("7fc00000","7fc00000")][scenario % 6]
        yield f"gbind {counted} {low} {high} {scenario % 8}"
        yield "gcache 9"
        for kind, input in [(9,7),(10,7),(9,8),(10,8),(9,9),(10,9),(9,10),(10,10)]:
            yield f"gunary {kind} {input}"
        yield "gcache 11"
        yield "gbinary 0 17 18"
        yield "gclamp 20 bf800000 3f800000"
        yield "ground 1 21 2"
        yield "gcache 20"
        yield "gunary 9 6"
        yield "gunary 10 6"
        yield "gunary 9 3"
        for node in range(27):
            yield f"ginfo {node}"
        for mode, roots in [(0,[11,12,13,14,15,16,24,25,26]),(1,list(range(11,27)))]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    for point in [[0,0,0],[-17,-64,32],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]:
                        yield f"query {sampler} " + " ".join(map(str,point))
                        yield f"counts {2880+scenario}"
                    for sizes, minimum, steps in [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4]),([1,3,1],[0,0,0],[1,1,1])]:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" 0 {scenario} 2"
                        yield f"counts {2880+scenario}"

    power_inputs = []
    for value in power_values:
        yield f"constant {value}"
        power_inputs.append(index)
        index += 1
    for mode in range(3):
        for base, base_bits in zip(power_inputs, power_values):
            for exponent, exponent_bits in zip(power_inputs, power_values):
                base_constant = number(struct.unpack(">f", bytes.fromhex(base_bits))[0])
                exponent_constant = number(struct.unpack(">f", bytes.fromhex(exponent_bits))[0])
                fields = [str(base),str(exponent)] if mode == 0 else [base_constant,str(exponent)] if mode == 1 else [str(base),exponent_constant]
                yield f"power {mode} " + " ".join(fields)
                yield f"query {index} -17 -64 32"
                yield f"volume {index} 2 3 2 -17 -64 32 4 8 4 {0 if mode == 0 else 2} {index} 2"
                index += 1
    for scenario in range(256):
        mode = scenario % 3
        child = rng.choice(all_children)
        constant = rng.choice(scales + [2.,-2.,3.,-3.,4.,-4.]) if scenario % 2 else struct.unpack(">d",rng.getrandbits(64).to_bytes(8,"big"))[0]
        fields = [str(child),str(rng.choice(all_children))] if mode == 0 else [number(constant),str(child)] if mode == 1 else [str(child),number(constant)]
        yield f"power {mode} " + " ".join(fields)
        yield from probes(index, points=8, exact=mode == 0)
        index += 1

    for scenario in range(64):
        yield "greset"
        for value in ["00000000","80000000","3f800000","bf800000","3f000000","bf000000","40000000","c0000000","40400000","c0400000","40800000","c0800000","7fc00000","7f800000","ff800000",power_values[scenario % len(power_values)]]:
            yield f"gconst {value}"
        yield "ggradient 1 0 -64 64 bf800000 3f800000"
        for slot, source in enumerate([rng.choice(noise_children),rng.choice(all_children)]):
            yield f"counted {source}"
            counted = index
            index += 1
            yield f"gbind {counted} bf800000 3f800000 {7 if slot == 0 else scenario % 8}"
        yield "gcache 17"
        yield "gpow 17 18"
        yield "gpow 15 18"
        for exponent in [4,5,2,3,6,7,8,9,10,11,12,13,14,0,1]:
            yield f"gpow 17 {exponent}"
        yield "gpow 15 15"
        yield "gpow 19 18"
        yield "gpow 18 19"
        yield "gcache 20"
        yield "gpow 40 21"
        yield "gclamp 41 bf800000 3f800000"
        yield "ginterp 42 4 8"
        yield "gsconstant 00000000"
        yield "gsconstant 3f800000"
        yield "gspoints 42 2 bf800000 00000000 0 3f800000 00000000 1"
        yield "gspline 2"
        yield "gsurface 42 2 -64 8"
        yield "gpow 12 19"
        yield "gcache 46"
        for node in range(48):
            yield f"ginfo {node}"
        for mode, roots in [(0,list(range(20,38))),(1,list(range(20,47)))]:
            for root in roots:
                if root == 44 and power_values[scenario % len(power_values)] in ["7fc00000","7fa00001"]:
                    yield f"gtry {mode} {root}"
                    continue
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for enabled in [0,1]:
                    yield f"context {enabled}"
                    for point in [[0,0,0],[-17,-64,32],[1,7,-1],[-2147483648,0,2147483647],[2147483647,-1,-2147483648]]:
                        yield f"query {sampler} " + " ".join(map(str,point))
                        for counter in [2944+scenario*2,2945+scenario*2]:
                            yield f"counts {counter}"
                    for sizes, minimum, steps in [([2,5,3],[-17,-64,32],[1,1,1]),([2,5,3],[-16,-64,32],[4,8,4])]:
                        yield f"volume {sampler} " + " ".join(map(str,sizes+minimum+steps)) + f" 0 {scenario} 2"
                        for counter in [2944+scenario*2,2945+scenario*2]:
                            yield f"counts {counter}"
        for root in [46,47]:
            for mode in [0,1,1]:
                yield f"gtry {mode} {root}"


    # Context fields are resolved at evaluation time. Deliberately bind values
    # outside declared ranges and axes to retain vanilla's compiler behavior.
    for scenario in range(16):
        sources = []
        for field in range(3):
            yield f"gradient {(scenario+field) % 3} {1 if scenario % 5 == 0 else 0} {-17 if scenario % 5 else 0} {32 if scenario % 5 else 0} {floats[(scenario+field) % len(floats)]} {floats[(scenario+field+5) % len(floats)]}"
            source = index
            index += 1
            yield f"counted {source}"
            sources.append(index)
            index += 1
        yield f"constant {floats[scenario % len(floats)]}"
        fallback = index
        index += 1
        roots = []
        for field in range(3):
            for default in [-1, fallback]:
                yield f"wfield {field} {default}"
                roots.append(index)
                index += 1
        for source in roots[:6]:
            yield f"blenddensity {source}"
            roots.append(index)
            index += 1
        for slot, source in enumerate(roots[:6]):
            yield f"cache {slot} {source}"
            roots.append(index)
            index += 1
        for root in roots:
            for cached in [0,1]:
                for blender in [0,1]:
                    for mask in [0,1,2,4,7]:
                        bindings = [str(sources[field] if mask & (1<<field) else -1) for field in range(3)]
                        yield f"wcontext {cached} " + " ".join(bindings) + f" {blender}"
                        for point in [[0,0,0],[-17,-64,32],[-17,-64,32],[2147483647,-2147483648,-1]]:
                            yield f"query {root} " + " ".join(map(str,point))
                        for tail in [0,2]:
                            yield f"volume {root} 2 5 3 -17 -64 32 4 8 4 {tail} {scenario} 2"
                        for counter in range(3072+scenario*3,3075+scenario*3):
                            yield f"counts {counter}"
        yield "greset"
        for field in range(3):
            yield f"gwfield {field}"
        yield "gwfield 0"  # Structurally equal enum leaf.
        yield "gsame 0 3"
        yield f"gconst {floats[scenario % len(floats)]}"
        for axis in range(3):
            yield f"ggradient {axis} 0 -17 32 bf800000 3f800000"
        yield "ggradient 1 1 0 0 bf800000 3f800000"  # Runtime failure.
        for input in [0,1,2,4,5,6,7,8]:
            yield f"gblenddensity {input}"  # Nodes 9..16.
        yield "gbinary 0 0 1"  # 17
        yield "gbinary 0 17 2"  # 18
        yield "gblenddensity 18"  # 19
        yield "gcache 19"  # 20
        yield "gcache 0"  # 21
        yield "gslice 1 17 9"  # 22
        yield "ginterp 19 4 8"  # 23
        yield "ground 1 0 4"  # 24
        yield "gpow 0 4"  # 25
        yield "gsconstant bf800000"
        yield "gsconstant 3f800000"
        yield "gspoints 0 2 00000000 3f000000 0 3f800000 bf000000 1"
        yield "gspline 2"  # 26
        yield "gcache 26"  # 27
        for node in range(28):
            yield f"ginfo {node}"
        for mode, graph_roots in [(0,[0,1,2,9,10,11,12,13,14,15,16,19,22,23,24,25,26]),(1,[0,1,2,9,10,11,12,13,14,15,16,19,20,21,22,23,24,25,26,27])]:
            for root in graph_roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for cached in [0,1]:
                    for mask, blender in [(0,0),(0,1),(1,0),(2,1),(4,0),(7,1)]:
                        bindings = [str(sources[field] if mask & (1<<field) else -1) for field in range(3)]
                        yield f"wcontext {cached} " + " ".join(bindings) + f" {blender}"
                        for point in [[0,0,0],[-17,-64,32],[-17,-64,32],[2147483647,-2147483648,-1]]:
                            yield f"query {sampler} " + " ".join(map(str,point))
                        for minimum, steps in [([-17,-64,32],[1,1,1]),([-16,-64,32],[4,8,4])]:
                            yield f"volume {sampler} 2 5 3 " + " ".join(map(str,minimum+steps)) + f" 0 {scenario} 2"
                        for counter in range(3072+scenario*3,3075+scenario*3):
                            yield f"counts {counter}"


    # Direct and named holders retain distinct equality, then INLINE_REFERENCE
    # runs before cache preparation. Nested wrappers remove just one outer level.
    for scenario in range(32):
        yield "greset"
        for value in ["80000000","3f000000","3f800000","40000000"]:
            yield f"gconst {value}"
        yield "ggradient 0 0 -64 64 bf800000 3f800000"  # 4
        yield "ggradient 1 0 -64 64 bf800000 3f800000"  # 5
        yield "gwfield 2"  # 6
        yield "gref 0"  # 7
        yield "gnamed minecraft:transport/a 0"  # 8
        yield "gnamed minecraft:transport/b 0"  # 9
        yield "glookup minecraft:transport/a"  # 10
        yield "gref 0"  # 11
        yield "gref 8"  # 12
        yield "gref 7"  # 13
        yield "gcache 4"  # 14
        yield "gnamed minecraft:transport/cache 14"  # 15
        yield "gref 14"  # 16
        yield "glookup minecraft:transport/cache"  # 17
        yield "gref 2"  # 18
        for input in [7,8,12,13]:
            yield f"gpow {input} 1"  # 19..22
        yield "gbinary 3 4 18"  # 23
        yield "ground 1 4 18"  # 24
        yield "gcache 15"  # 25
        yield "gcache 16"  # 26
        yield "gcache 17"  # 27
        yield "gbinary 0 25 26"  # 28
        yield "gnamed minecraft:transport/y 5"  # 29
        yield "gref 29"  # 30
        yield "gbinary 0 29 30"  # 31
        yield "gblenddensity 31"  # 32
        yield "gcache 32"  # 33
        yield "gnamed minecraft:transport/beard 6"  # 34
        yield "gref 34"  # 35
        yield "gsconstant bf800000"
        yield "gsconstant 3f800000"
        yield "gspoints 30 2 bf800000 3f000000 0 3f800000 bf000000 1"
        yield "gspline 2"  # 36
        yield "gnamed minecraft:transport/spline 36"  # 37
        yield "ginterp 37 4 8"  # 38
        yield "gref 38"  # 39
        yield "ggradient 1 1 0 0 bf800000 3f800000"  # 40
        yield "gnamed minecraft:transport/failure 40"  # 41
        yield "gcache 41"  # 42
        for node in range(43):
            yield f"ginfo {node}"
        for left,right in [(7,11),(8,10),(8,9),(7,8),(12,13),(15,17),(25,26),(25,27),(29,30),(34,35),(36,37),(38,39)]:
            yield f"gsame {left} {right}"
        for mode, roots in [(0,[7,8,9,10,12,13,18,19,20,21,22,23,24,29,30,31,32,34,35,36,37,38,39,41]),(1,list(range(7,40))+[41,42])]:
            for root in roots:
                yield f"gcompile {mode} {root}"
                sampler = index
                index += 1
                for cached in [0,1]:
                    for beard, blender in [(-1,0),(rng.choice(all_children),1)]:
                        yield f"wcontext {cached} -1 -1 {beard} {blender}"
                        for point in [[0,0,0],[-17,-64,32],[1,7,-1],[2147483647,-2147483648,-1]]:
                            yield f"query {sampler} " + " ".join(map(str,point))
                        for minimum, steps in [([-17,-64,32],[1,1,1]),([-16,-64,32],[4,8,4])]:
                            yield f"volume {sampler} 2 5 3 " + " ".join(map(str,minimum+steps)) + f" 0 {scenario} 2"
        for root in [14,15,16,17,25,26,27,28,33,42]:
            yield f"gtry 0 {root}"



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    build = lab.LOCAL / "rust-density-parity"
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / "report.json"
    report_path.unlink(missing_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = test_rust_parity.verify_artifacts()
    commands = list(corpus())
    inputs = build / "inputs.txt"
    inputs.write_text("\n".join(commands) + "\n", encoding="utf-8")
    source = lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDensityOracle.java"
    sources = [source, Path(__file__), lab.ROOT / "tools/test_rust_parity.py", lab.ROOT / "tools/lab.py",
               *sorted((lab.ROOT / "crates/horizon-core").rglob("*.rs")),
               lab.ROOT / "crates/horizon-core/Cargo.toml", lab.ROOT / "Cargo.toml", lab.ROOT / "Cargo.lock"]
    tested_sources = test_rust_parity.source_hashes(sources)
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    dependency_path = os.pathsep.join(map(str, libraries))
    run = test_rust_parity.run
    run(["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none",
         "-cp", os.pathsep.join([str(jars["server"]), dependency_path]), "-d", str(classes), str(source)])
    run(["cargo", "build", "--release", "--bin", "horizon-density-parity"])
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=lab.ROOT, text=True))
    binary = Path(metadata["target_directory"]) / "release" / ("horizon-density-parity.exe" if os.name == "nt" else "horizon-density-parity")
    candidate = build / "rust.txt"
    with inputs.open("rb") as stdin, candidate.open("wb") as stdout:
        run([str(binary)], stdin=stdin, stdout=stdout)
    references = []
    for side, jar in jars.items():
        reference = build / f"vanilla-{side}.txt"
        run(["java", "-Xmx512M", "-cp", os.pathsep.join([str(classes), str(jar), dependency_path]),
             "dev.horizonlab.parity.VanillaDensityOracle", str(inputs), str(reference)])
        matched = test_rust_parity.compare(reference, candidate)
        references.append({"side": side, "jar_sha256": lab.digest(jar, "sha256"),
                           "output_sha256": lab.digest(reference, "sha256"), "matched_output_lines": matched})
        print(f"{side}: {matched:,} density output lines match Rust", flush=True)
    test_rust_parity.ensure_sources_unchanged(sources, tested_sources)
    report = {"schema": 1, "minecraft": lock["minecraft"]["version"], "platform": platform.platform(),
              "java": subprocess.check_output(["java", "--version"], text=True).splitlines()[0],
              "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
              "manifest_sha1": lock["minecraft"]["manifest_sha1"], "commands": dict(Counter(line.split()[0] for line in commands)),
              "inputs_sha256": lab.digest(inputs, "sha256"), "rust_output_sha256": lab.digest(candidate, "sha256"),
              "sources_sha256": tested_sources, "references": references,
              "scope": "Compiled constant, clamped/repeated/mirrored gradient, unshifted and XZ/XYZ shifted noise, all eleven vanilla unary operators and constant-operand specializations, binary arithmetic/min/max, power with constant base/exponent samplers, floor/nearest/ceil/truncate rounding to integer or sampled multiples, range choice and linear interpolation with constant-branch/endpoint specializations; multi-threshold and single-threshold selection; shift/shift-A/shift-B offset samplers; old blended terrain noise definition ranges, ordered smeared FBM construction, scalar/bulk composition and failure RNG state; distance metrics and distance-to-point samplers with wrapped differences; cached and uncached density contexts, nested cache nodes, scalar/volume hits, packed-coordinate key collisions, NaN misses and zero-initialized failure buffers, exact lattice lookup; X/Y/Z and combined XZ slices, reduced-volume bulk evaluation, unchanged-volume delegation, nested and cached children; positive-cell grid interpolation with aligned delegation, eight-corner scalar sampling, repeated-addition bulk columns and dense-then-strided selection; raw and Y=0-sliced surface-height search with scalar density probes, bulk upper bounds, strict positive crossing, saturating floor conversion and wrapped cell alignment, early lower-bound return and single-layer validation; nested cubic splines with endpoint extrapolation, binary interval selection, compact coordinate IDs, lazy scalar/volume coordinate caches, repeated NaN scalar misses and partial output failures, constant-NaN compile rejection and conservative spline range propagation; interval construction, hulls, arithmetic/min/max, reciprocal, abs/square/sign, powers and logarithms, clamp, monotonic endpoint mapping, finite/infinite interpolation, predicates, equality and Java hashes; scalar and strided bulk paths, signed/wrapped integer coordinates, nested child bulk evaluation, unary/leaf buffer tails, exact-volume multi-input buffers, scalar short circuits, NaN/signed-zero differences between scalar and bulk min/max, arithmetic failures and partial writes. Finite outputs and signed zeros compared by bits; NaNs canonicalized. Initial integrated density graph compiler for constants, gradients, all eleven unary and six binary operations, power, rounding, clamp, range choice, lerp, interval selection, distance-to-point, slices, caches, interpolation, surface search, nested cubic splines, registered normal and shifted noise, offset noise, old blended noise and outer End island density: structural interning, range/axis metadata, constant and disjoint-range specialization, uniform-axis slicing, cache-ID allocation/deduplication, memoized roots and failed preparation state are compared with vanilla. Seeded normal-noise lookup, modern/legacy positional factories, named instance reuse, dedicated Nether legacy branches and old blended terrain seed selection are checked against vanilla RandomState's actual compile context. Power samplers retain base/exponent evaluation order, literal-base precedence, sqrt/identity/square/cube exponent specialization, negative-exponent reciprocal composition for sqrt/identity/square/cube and the direct signed-exponent fallback, f64 constant operands, buffer tails and partial writes. Power ranges check exact and interval exponents, signed-zero bases, negative bases with integer exponent bounds, infinite exponents and NaI hulls. Log and square-root graphs check partially invalid domains, range clipping, cache preparation, scalar/bulk composition and trusted bound metadata. Rounding checks positive-infinity ties, int saturation, signed zeros, NaNs, infinite and zero multiples, literal-one specialization, dynamic-multiple child order, declared range propagation, cache preparation and partial input writes on multiple failure. End islands use the dedicated legacy world-seed stream, 17,292 discarded draws, simplex island threshold, truncated negative coordinates, wrapped integer edges, vertical bulk columns and nested/cache/spline/interpolation graphs. Context-bound alpha, offset and structure-density samplers compare fallback/custom fallback, missing/bound fields, declared metadata independent of bindings, same-context delegation, scalar/bulk child calls, failure writes, constant tails, cache namespaces, uniform-axis slicing and nested spline/arithmetic/interpolation graphs. Blend-density wrappers compare absent and actual EMPTY vanilla Blender contexts, input-first evaluation and input metadata propagation; nonempty old-world Blender math remains pending. Direct and registered density holders compare structural value versus holder identity, named lookup, raw delegation and metadata, one-level reference inlining before cache detection, nested holder rewrite replacement, constant specialization, cache allocation/deduplication and memoized roots. Remaining graph definitions, remaining range integration, codecs, world-dependent samplers and terrain integration remain pending; no complete terrain, gameplay, rendering or performance claim."}
    lab.write_json(report_path, report)
    print(f"Scoped density parity passed: {report_path}")


if __name__ == "__main__":
    main()
