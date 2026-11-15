#!/usr/bin/env python3
"""Capture baseline hashes from actual compiled layout symbols, not cloned enums.

This is an explicit developer diagnostic, never run by the golden-test CI step.
It compiles a generated unsafe diagnostic ONLY under --work-dir (use target/).
No Cargo command is invoked. Match rustc, archive and dependency artifacts exactly.
"""
import argparse
import json
from pathlib import Path
import re
import shlex
import subprocess

from generate_wire_enum_goldens import discover


def symbol_map(raw, demangled, identities):
    if len(raw) != len(demangled):
        raise ValueError("nm output mismatch")
    result = {}
    for original, readable in zip(raw, demangled):
        match = re.search(r"<([\w:]+) as mod_vocab::wire::Wire>::layout$", readable)
        if not match:
            continue
        path = match[1].replace("mod_vocab::", "crate::", 1)
        if path not in identities:
            continue
        if path in result:
            raise ValueError(f"duplicate layout symbol: {path}")
        result[path] = original.split()[-1]
    if set(result) != set(identities):
        raise ValueError(f"missing layout symbols: {sorted(set(identities) - set(result))}")
    return result


def diagnostic_source(symbols, externs):
    source = "".join(f"extern crate {name};\n" for name in externs)
    for index, symbol in enumerate(symbols.values()):
        source += f'unsafe extern "Rust" {{ #[link_name="{symbol}"] fn f{index}(v: &mut Vec<&\'static str>) -> u64; }}\n'
    source += "fn main() {\n"
    for index, path in enumerate(symbols):
        source += f'println!("{path} {{:016x}}", unsafe {{ f{index}(&mut vec![]) }});\n'
    return source + "}\n"


def build_inputs(log):
    lines = [line for line in log.splitlines() if "Running `" in line and "--crate-name mod_vocab " in line]
    if len(lines) != 1:
        raise ValueError("capture needs exactly one recorded vocab rustc invocation")
    args = shlex.split(lines[0].split("`", 1)[1].rsplit("`", 1)[0])
    directory = Path(args[args.index("--out-dir") + 1])
    suffix = next(arg.split("=", 1)[1] for arg in args if arg.startswith("extra-filename="))
    externs = [args[i + 1].replace(".rmeta", ".rlib") for i, arg in enumerate(args) if arg == "--extern" and args[i + 1].endswith((".rmeta", ".rlib"))]
    return directory / f"libmod_vocab{suffix}.rlib", directory, externs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expanded", type=Path, required=True)
    parser.add_argument("--build-log", type=Path, help="cargo -vv log containing the actual vocab rustc command")
    parser.add_argument("--library", type=Path)
    parser.add_argument("--dependency-dir", type=Path)
    parser.add_argument("--extern", action="append", default=[], help="NAME=matching.rlib; repeat for direct runtime dependencies, not the proc macro")
    parser.add_argument("--base", required=True)
    parser.add_argument("--work-dir", type=Path, default=Path("target/wire-layout-capture"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--identities-output", type=Path, required=True)
    args = parser.parse_args()
    if args.build_log:
        args.library, args.dependency_dir, args.extern = build_inputs(args.build_log.read_text())
    if args.library is None or args.dependency_dir is None:
        parser.error("supply --build-log or --library and --dependency-dir")
    identities = discover(args.expanded.read_text())
    raw = subprocess.check_output(["nm", "--defined-only", str(args.library)], text=True, stderr=subprocess.DEVNULL).splitlines()
    demangled = subprocess.check_output(["nm", "-C", "--defined-only", str(args.library)], text=True, stderr=subprocess.DEVNULL).splitlines()
    symbols = dict(sorted(symbol_map(raw, demangled, identities).items()))
    args.work_dir.mkdir(parents=True, exist_ok=True)
    source = args.work_dir / "capture.rs"
    source.write_text(diagnostic_source(symbols, [item.split("=", 1)[0] for item in args.extern]))
    binary = args.work_dir / "capture"
    command = ["rustc", "--edition=2024", "--crate-name", "layout_capture", "-L", f"dependency={args.dependency_dir}"]
    for item in args.extern:
        command += ["--extern", item]
    command += ["-C", f"link-arg={args.library}", str(source), "-o", str(binary)]
    (args.work_dir / "command.json").write_text(json.dumps(command, indent=2))
    subprocess.run(command, check=True)
    output = subprocess.check_output([str(binary.resolve())], text=True)
    (args.work_dir / "hashes.txt").write_text(output)
    hashes = dict(line.split() for line in output.splitlines())
    if set(hashes) != set(symbols):
        raise ValueError("capture identity mismatch")
    manifest = {"base": args.base, "configuration": "native default features, normal dev opt0, before fieldless const fold", "enums": [
        {"name": path.split("::")[-1], "path": path, "hash": hashes[path], "canonical_path": path}
        for path in symbols]}
    for entry in manifest["enums"]:
        if entry["canonical_path"] == "crate::generated::HookId":
            entry["baseline_hash"] = entry.pop("hash")
            entry["comparison"] = "same_source_generated_variants"
    args.output.write_text(json.dumps(manifest, indent=2) + "\n")
    args.identities_output.write_text(json.dumps(sorted(symbols), indent=2) + "\n")


if __name__ == "__main__":
    main()
