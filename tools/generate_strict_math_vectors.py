#!/usr/bin/env python3
"""Generate portable literal StrictMath vectors using only public JDK APIs.

Run from any directory: python3 tools/generate_strict_math_vectors.py
Requires Python 3.11+ and a JDK with Java source-file launching (11+), not
Minecraft, prepared resources, Cargo, or any third-party Python package.
Use --java /path/to/java to select a reference JDK. The exact java -version
output is recorded in the generated file. Regeneration is byte-for-byte
repeatable with the same JDK: inputs use the explicitly defined SplitMix64
sequence below, not Python's version-dependent random distributions.

The embedded Java oracle is original harness code. It calls public APIs;
no JDK implementation sources are read, copied, or needed. Expected NaNs
are classification expectations only: their payload/sign are not portable.
"""

import argparse
import struct
import subprocess
import tempfile
from pathlib import Path


SEED = 0x3668_5A17_C0DE_2026
MASK = (1 << 64) - 1
SIGN = 1 << 63
MIN_CASES = 768
DEFAULT_OUTPUT = (
    Path(__file__).resolve().parents[1]
    / "crates/horizon-mod-wasm/tests/fixture/src/strict_math_vectors.rs"
)

JAVA_ORACLE = r"""
import java.io.BufferedReader;
import java.io.InputStreamReader;

class StrictMathVectorOracle {
    public static void main(String[] args) throws Exception {
        BufferedReader input = new BufferedReader(new InputStreamReader(System.in));
        String line;
        StringBuilder output = new StringBuilder();
        while ((line = input.readLine()) != null) {
            String[] fields = line.split(" ");
            double x = Double.longBitsToDouble(Long.parseUnsignedLong(fields[1], 16));
            double result;
            switch (fields[0]) {
                case "sin": result = StrictMath.sin(x); break;
                case "cos": result = StrictMath.cos(x); break;
                case "tan": result = StrictMath.tan(x); break;
                case "exp": result = StrictMath.exp(x); break;
                case "pow":
                    double y = Double.longBitsToDouble(Long.parseUnsignedLong(fields[2], 16));
                    result = StrictMath.pow(x, y);
                    break;
                default: throw new IllegalArgumentException(fields[0]);
            }
            output.append(Long.toHexString(Double.doubleToRawLongBits(result))).append('\n');
        }
        System.out.print(output);
    }
}
"""


class SplitMix64:
    def __init__(self, seed):
        self.state = seed

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        value = self.state
        value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & MASK
        return value ^ (value >> 31)


def bits(value):
    return struct.unpack(">Q", struct.pack(">d", value))[0]


class Cases:
    """Insertion-ordered unique bit tuples with optional named edge labels."""

    def __init__(self):
        self.rows = {}

    def add(self, *inputs, label=""):
        self.rows.setdefault(tuple(inputs), label)

    def around(self, value, label, radius=2):
        center = bits(value)
        for offset in range(-radius, radius + 1):
            self.add(center + offset, label=f"{label}; bit offset {offset:+}")


def ieee_edges():
    return [
        ("positive zero", 0x0000000000000000),
        ("negative zero", 0x8000000000000000),
        ("minimum positive subnormal", 0x0000000000000001),
        ("minimum negative subnormal", 0x8000000000000001),
        ("maximum positive subnormal", 0x000FFFFFFFFFFFFF),
        ("maximum negative subnormal", 0x800FFFFFFFFFFFFF),
        ("minimum positive normal", 0x0010000000000000),
        ("minimum negative normal", 0x8010000000000000),
        ("maximum positive finite", 0x7FEFFFFFFFFFFFFF),
        ("maximum negative finite", 0xFFEFFFFFFFFFFFFF),
        ("positive infinity", 0x7FF0000000000000),
        ("negative infinity", 0xFFF0000000000000),
        ("quiet NaN", 0x7FF8000000000000),
        ("negative payload NaN", 0xFFF8123456789ABC),
        ("signaling NaN", 0x7FF0000000000001),
    ]


def unary_cases(operation):
    cases = Cases()
    for label, value in ieee_edges():
        cases.add(value, label=label)
    if operation in ("sin", "cos", "tan"):
        # Literal binary64 landmarks: these are inputs, never expected outputs.
        landmarks = [
            ("tiny kernel boundary", "0x1p-27"),
            ("smaller tiny kernel boundary", "0x1p-28"),
            ("pi/4 kernel boundary", "0x1.921fb54442d18p-1"),
            ("pi/2 pole or extremum", "0x1.921fb54442d18p+0"),
            ("pi zero or extremum", "0x1.921fb54442d18p+1"),
            ("3*pi/2 pole or extremum", "0x1.2d97c7f3321d2p+2"),
            ("2*pi zero or extremum", "0x1.921fb54442d18p+2"),
            ("medium reduction boundary", "0x1.921fb54442d18p+19"),
            ("huge reduction 2^100", "0x1p+100"),
            ("huge reduction 2^500", "0x1p+500"),
            ("huge reduction 2^1000", "0x1p+1000"),
        ]
        for label, value in landmarks:
            cases.around(float.fromhex(value), label)
            cases.around(-float.fromhex(value), "negative " + label)
        # Sweep the full exponent range of large argument reduction, both signs.
        for exponent in range(20, 1024, 17):
            for mantissa in (0, 0x921FB54442D18, 0xFFFFFFFFFFFFF):
                value = ((exponent + 1023) << 52) | mantissa
                cases.add(value, label=f"large reduction exponent {exponent}")
                cases.add(value | SIGN, label=f"negative large reduction exponent {exponent}")
    else:
        landmarks = [
            ("exp overflow threshold", "0x1.62e42fefa39efp+9"),
            ("exp zero underflow threshold", "-0x1.74910d52d3051p+9"),
            ("exp minimum normal transition", "-0x1.6232bdd7abcd2p+9"),
            ("exp minimum subnormal transition", "-0x1.74385446d71c3p+9"),
            ("positive half ln2 reduction", "0x1.62e42fefa39efp-2"),
            ("negative half ln2 reduction", "-0x1.62e42fefa39efp-2"),
            ("positive 1.5 ln2 reduction", "0x1.0a2b23f3bab73p+0"),
            ("negative 1.5 ln2 reduction", "-0x1.0a2b23f3bab73p+0"),
            ("tiny exp boundary", "0x1p-28"),
        ]
        for label, value in landmarks:
            cases.around(float.fromhex(value), label, radius=4)
        for value in (-1000.0, -746.0, -745.0, -744.0, -740.0, -710.0,
                      -709.0, -708.0, -1.0, 0.5, 1.0, 2.0, 709.0, 710.0, 1000.0):
            cases.add(bits(value), label=f"exp range {value:g}")
    # Share trig input sets; exp has a separate deterministic stream.
    rng = SplitMix64(SEED ^ (0xE000 if operation == "exp" else 0x7100))
    index = 0
    while len(cases.rows) < MIN_CASES:
        selector = index % 4
        if selector == 0:
            value = rng.next()  # Full binary64 spread, including huge arguments.
        elif selector == 1:
            value = bits((int(rng.next() % 65537) - 32768) / 32.0)
        elif selector == 2 and operation == "exp":
            value = bits(-750.0 + (rng.next() % 1500001) / 1000.0)
        elif selector == 2:
            value = ((rng.next() % 2047) << 52) | (rng.next() & ((1 << 52) - 1))
            value |= rng.next() & SIGN
        else:
            value = (rng.next() & ((1 << 52) - 1)) | (rng.next() & SIGN)
        cases.add(value)
        index += 1
    return cases


def pow_cases():
    cases = Cases()
    edges = ieee_edges() + [
        ("one", bits(1.0)), ("minus one", bits(-1.0)),
        ("two", bits(2.0)), ("minus two", bits(-2.0)),
        ("half", bits(0.5)), ("minus half", bits(-0.5)),
        ("three", bits(3.0)), ("minus three", bits(-3.0)),
    ]
    for x_label, x in edges:
        for y_label, y in edges:
            cases.add(x, y, label=f"{x_label} ^ {y_label}")
    # Integer classification at 2^31, 2^52, 2^53 and huge-y shortcuts.
    exponents = [
        -1075.0, -1074.0, -1073.0, -1024.0, -1023.0, -1022.0,
        1023.0, 1024.0, 1074.0, 1075.0,
        2147483647.0, 2147483648.0, 2147483649.0,
        4503599627370495.0, 4503599627370496.0, 4503599627370497.0,
        9007199254740991.0, 9007199254740992.0, 9007199254740994.0,
    ]
    for x in (0.5, -0.5, 2.0, -2.0, 1.0000000000000002, 0.9999999999999999):
        for y in exponents:
            for sign in (1.0, -1.0):
                cases.add(bits(x), bits(y * sign), label="integer parity / overflow / underflow")
    for x in (0.25, 0.75, 1.5, 3.0, -0.25, -0.75, -1.5, -3.0):
        for y in (0.25, -0.25, 1.5, -1.5, 2.5, -2.5, 31.0, 32.0, 33.0):
            cases.add(bits(x), bits(y), label="noninteger / odd / even exponent")
    for x in (0x3FEFFFFFFFFFFFFF, 0x3FF0000000000001,
              0xBFEFFFFFFFFFFFFF, 0xBFF0000000000001):
        for y in (bits(1e20), bits(-1e20), bits(1e100), bits(-1e100)):
            cases.add(x, y, label="near unit base with huge exponent")
    rng = SplitMix64(SEED ^ 0xB0A0)
    # Add 512 seeded cases in addition to the extensive named Cartesian edges.
    target = len(cases.rows) + 512
    index = 0
    while len(cases.rows) < target:
        selector = index % 4
        if selector == 0:
            x, y = rng.next(), rng.next()
        elif selector == 1:
            x = bits((rng.next() % 32769 + 1) / 1024.0)
            y = bits((int(rng.next() % 32769) - 16384) / 256.0)
        elif selector == 2:
            x = bits(-(rng.next() % 32769 + 1) / 1024.0)
            y = bits(float(int(rng.next() % 129) - 64))
        else:
            x = bits((rng.next() % 8193 + 1) / 4096.0)
            y = bits((int(rng.next() % 2200001) - 1100000) / 1000.0)
        cases.add(x, y)
        index += 1
    return cases


def generate(java):
    version = subprocess.run(
        [java, "-version"], check=True, capture_output=True, text=True
    )
    version_lines = (version.stdout + version.stderr).strip().splitlines()
    tables = {name: unary_cases(name) for name in ("sin", "cos", "tan", "exp")}
    tables["pow"] = pow_cases()
    requests = [
        name + " " + " ".join(f"{value:016x}" for value in inputs)
        for name, cases in tables.items()
        for inputs in cases.rows
    ]
    with tempfile.TemporaryDirectory(prefix="horizon-strict-vectors-") as directory:
        oracle = Path(directory) / "StrictMathVectorOracle.java"
        oracle.write_text(JAVA_ORACLE, encoding="utf-8")
        result = subprocess.run(
            [java, str(oracle)], input="\n".join(requests) + "\n",
            check=True, capture_output=True, text=True,
        )
    expected = result.stdout.splitlines()
    if len(expected) != len(requests):
        raise RuntimeError(f"Oracle returned {len(expected)} rows for {len(requests)} requests")
    lines = [
        "// @generated by tools/generate_strict_math_vectors.py; do not edit.",
        "// Independent reference: public java.lang.StrictMath APIs, no vanilla data.",
        "// Exact reference JDK (java -version):",
        *("// " + line for line in version_lines),
        f"// Seed: 0x{SEED:016x}; explicitly specified SplitMix64 input sequence.",
        "// Rows contain raw binary64 bits: (x, expected) or (x, y, expected).",
        "// Compare all non-NaNs bit-for-bit, including signed zero. For expected",
        "// NaNs assert is_nan() only; NaN payload and sign are not contractual.",
        "// Tables have no platform dependencies and are shared by native/wasm tests.",
        "",
    ]
    cursor = 0
    for name, cases in tables.items():
        fields = "u64, u64, u64" if name == "pow" else "u64, u64"
        lines.append("#[rustfmt::skip]")
        lines.append(f"pub(crate) const {name.upper()}: &[({fields})] = &[")
        pending = []
        for inputs, label in cases.rows.items():
            output = int(expected[cursor], 16)
            cursor += 1
            row = "(" + ", ".join(f"0x{value:016x}" for value in (*inputs, output)) + "),"
            if label:
                row += f" /* {label} */"
            pending.append(row)
            if len(pending) == 2:
                lines.append("    " + " ".join(pending))
                pending = []
        if pending:
            lines.append("    " + " ".join(pending))
        lines.extend([
            "];", f"// {name}: {len(cases.rows)} independent reference rows.", "",
        ])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java", default="java", help="reference JDK java executable")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    args.output.write_text(generate(args.java), encoding="utf-8")
    print(f"Generated {args.output}")


if __name__ == "__main__":
    main()
