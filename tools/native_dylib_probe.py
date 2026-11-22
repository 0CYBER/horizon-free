#!/usr/bin/env python3
"""Dev-only exact-build Rust dylib probe. Build SDK with prebuilt_game.py first.

Uses SDK metadata only: never rebuilds mod_api or the game.
All compiler commands and native dependency inspection are retained in --output.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

import prebuilt_game as sdk_tools

FIXTURES = Path(__file__).with_suffix("")
PINNED = sdk_tools.ROOT / "tools/ci/rust-version"
# The allocator shim's entry points. Matched as name suffixes: mangled names that merely mention `alloc`
# (backtrace frames over `alloc::vec`) are not an allocator.
ALLOCATOR = tuple(f"___rust_{kind}" for kind in ("alloc", "dealloc", "realloc", "alloc_zeroed"))


def pinned_release():
    return PINNED.read_text().strip()


def pe_symbols(path):
    """(exported names, {imported DLL: names}) from a PE image's export and import directories."""
    data = path.read_bytes()
    u16 = lambda at: int.from_bytes(data[at:at + 2], "little")
    u32 = lambda at: int.from_bytes(data[at:at + 4], "little")
    pe = u32(0x3C)
    optional = pe + 24
    wide = u16(optional) == 0x20B
    directories = optional + (112 if wide else 96)
    sections = optional + u16(pe + 20)
    spans = [(u32(at + 12), max(u32(at + 8), u32(at + 16)), u32(at + 20))
             for at in (sections + 40 * i for i in range(u16(pe + 6)))]

    def offset(rva):
        return next(raw + rva - address for address, size, raw in spans if address <= rva < address + size)

    def string(rva):
        start = offset(rva)
        return data[start:data.index(b"\0", start)].decode()

    exported = []
    if export_rva := u32(directories):
        directory = offset(export_rva)
        names = offset(u32(directory + 32))
        exported = [string(u32(names + 4 * i)) for i in range(u32(directory + 24))]
    imported = {}
    if import_rva := u32(directories + 8):
        descriptor = offset(import_rva)
        while u32(descriptor + 12):
            thunk = offset(u32(descriptor) or u32(descriptor + 16))
            width, ordinal = (8, 1 << 63) if wide else (4, 1 << 31)
            names = []
            while entry := int.from_bytes(data[thunk:thunk + width], "little"):
                if not entry & ordinal:
                    names.append(string((entry & 0x7FFFFFFF) + 2))
                thunk += width
            imported[string(u32(descriptor + 12))] = names
            descriptor += 20
    return exported, imported


def windows_allocator(images, game, evidence):
    """Windows binds each import to the named DLL (no ELF interposition): exactly one image may export the Rust
    allocator, and every image calling it must import it from that one (arismoko/horizon#2394)."""
    rows = {}
    for image in images:
        exported, imported = pe_symbols(image)
        rows[image.name] = {
            "exports": [name for name in exported if name.endswith(ALLOCATOR)],
            "imports": {dll: [n for n in names if n.endswith(ALLOCATOR)] for dll, names in imported.items()
                        if any(n.endswith(ALLOCATOR) for n in names)},
        }
    evidence.write_text(json.dumps(rows, indent=2))
    definitions = [name for name, row in rows.items() if row["exports"]]
    if len(definitions) != 1:
        raise ValueError(f"expected one image exporting the Rust allocator, found {definitions}")
    foreign = {name: list(row["imports"]) for name, row in rows.items() if set(row["imports"]) - set(definitions)}
    if foreign:
        raise ValueError(f"allocator imported from a DLL other than {definitions[0]}: {foreign}")
    return rows


def preflight(sdk, compiler):
    identity = json.loads((sdk / sdk_tools.IDENTITY).read_text())
    if compiler.strip() != identity["rustc"].strip():
        raise ValueError("full rustc -vV mismatch")
    if f"release: {pinned_release()}\n" not in compiler:
        raise ValueError(f"probe requires the launcher's pinned rustc {pinned_release()}")
    for name, expected in identity["files"].items():
        path = sdk / name
        if not path.resolve().is_relative_to(sdk.resolve()):
            raise ValueError("SDK artifact escapes root")
        if sdk_tools.sha256(path) != expected:
            raise ValueError(f"exact SDK artifact mismatch: {name}")
    return identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    sdk, output = args.sdk.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    compiler = subprocess.check_output(["rustc", "-vV"], text=True)
    (output / "rustc.txt").write_text(compiler)
    identity = preflight(sdk, compiler)
    # Negative preflight must refuse before any library can execute.
    try:
        preflight(sdk, compiler + "different build\n")
    except ValueError:
        pass
    else:
        raise AssertionError("compiler mismatch was accepted")
    (output / "sdk-identity.json").write_text(json.dumps(identity, indent=2))
    deps = sdk / "deps"
    windows = sys.platform == "win32"
    extension, prefix = (".dll", "") if windows else (".so", "lib")

    def extern(crate):
        return sdk_tools.extern_args(deps, crate)

    def run(command, **kwargs):
        if command[0] == "rustc" and any(str(arg).startswith("horizon_client=") for arg in command):
            # Engine metadata describes code carried by the top-level host dylib.
            # Explicitly link that carrier in probes which otherwise only name internals.
            command = list(command)
            index = next(i for i, arg in enumerate(command) if str(arg).endswith(".rs"))
            source = Path(command[index])
            lines = source.read_text().splitlines(keepends=True)
            insertion = next((i for i, line in enumerate(lines) if line.strip() and not line.startswith(("//", "#!["))), len(lines))
            lines.insert(insertion, "extern crate horizon_mod_host;\n")
            copied = output / (source.parent.name + "-" + source.name)
            copied.write_text("".join(lines))
            command[index] = str(copied)
            command.extend(extern("horizon_mod_host"))
        with (output / "commands.log").open("a") as log:
            log.write(json.dumps(list(map(str, command))) + "\n")
            log.flush()
            if command[0] == "rustc":
                for driver in ("rustc", "clippy-driver"):
                    gate = [driver, *command[1:], "--emit=metadata", "-Dwarnings"]
                    gate[gate.index("-o") + 1] = str(output / "gate.rmeta")
                    log.write(json.dumps(gate) + "\n")
                    log.flush()
                    subprocess.run(gate, check=True, stdout=log, stderr=subprocess.STDOUT)
            subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, **kwargs)

    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    host = next(line.split(": ", 1)[1] for line in compiler.splitlines() if line.startswith("host: "))
    runtime = sysroot / "lib/rustlib" / host / "lib"
    std_runtime = sorted(runtime.glob(f"{prefix}std-*{extension}"))
    game = sdk_tools.dylib(deps, "horizon_mod_host")
    failures = []

    def runtime_env(*folders):
        variable = "PATH" if windows else "LD_LIBRARY_PATH"
        return {**os.environ, variable: os.pathsep.join(map(str, [*folders, deps, runtime])) + os.pathsep + os.environ.get(variable, "")}

    # 1. Counting allocator across a dylib boundary, standalone: the same compiler and flags as the prebuilt route.
    counting = output / "allocator"
    counting.mkdir(exist_ok=True)
    plain = ["rustc", "--edition=2024", "-C", "prefer-dynamic", "-C", "opt-level=3", "-L", f"dependency={counting}"]
    for crate, source, extra in [("game", "game.rs", []), ("modx", "modx.rs", ["--extern", f"game={counting / (prefix + 'game' + extension)}"])]:
        run([*plain, "--crate-type=dylib", "--crate-name", crate, *extra, str(FIXTURES / "allocator" / source),
             "-o", str(counting / (prefix + crate + extension))])
    counting_exe = counting / ("counting.exe" if windows else "counting")
    run([*plain, "--crate-name=counting", "--extern", f"game={counting / (prefix + 'game' + extension)}",
         "--extern", f"modx={counting / (prefix + 'modx' + extension)}", str(FIXTURES / "allocator/main.rs"), "-o", str(counting_exe)])
    report = subprocess.run([str(counting_exe)], env=runtime_env(counting), capture_output=True, text=True)
    (counting / "report.txt").write_text(report.stdout + report.stderr)
    print(f"counting allocator (exit {report.returncode}):\n{report.stdout}{report.stderr}")
    # On Windows this records arismoko/horizon#2394's cause: a `#[global_allocator]` in a dylib beside std's DLL splits from it.
    # The shipped route's checks below decide the result.
    if report.returncode != 0 or ("ONE ALLOCATOR" not in report.stdout and not windows):
        failures.append("cross-image allocations do not all go through the game's counting allocator (allocator/report.txt)")

    # 2. The shipped route: the starter mod and the generated executable, with the launcher's flags.
    shipped = output / "shipped"
    shipped.mkdir(exist_ok=True)
    route = ["rustc", "--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=3", "-L", f"dependency={deps}",
             "-L", f"dependency={sdk / 'host'}", "-L", f"native={sdk / 'native'}", "-L", f"dependency={shipped}"]
    starter = (sdk_tools.ROOT / "crates/cargo-horizon-mod/template/native/lib.rs").read_text()
    (shipped / "mod_0.rs").write_text(starter.replace("{id}", "probe"))
    subprocess.run([*route, "--crate-type", "rlib", "--crate-name", "mod_0", *extern("mod_api"),
                    "--out-dir", str(shipped), str(shipped / "mod_0.rs")], check=True)
    template = (sdk / sdk_tools.TEMPLATE).read_text()
    (shipped / "main.rs").write_text(template.replace("/* MOD_ENTRIES */", '        "probe" => mod_0::hooks,\n').replace(
        "/* BUILT_MODS */",
        '    mod_api::__private::native::BuiltMod { id: "probe", version: "0.1.0", side: mod_api::Side::Both },\n'))
    shipped_exe = shipped / ("horizon-client.exe" if windows else "horizon-client")
    subprocess.run([*route, "--crate-type", "bin", "--crate-name", "horizon_native_game",
                    *(flag for arg in identity["link_args"] for flag in ("-C", f"link-arg={arg}")),
                    *extern("horizon_client"), *extern("horizon_mod_host"), *extern("mod_api"), *extern("horizon_mod_package"),
                    "--extern", f"mod_0={shipped / 'libmod_0.rlib'}", "-o", str(shipped_exe), str(shipped / "main.rs")], check=True)
    if windows:
        # std's DLL always exports the allocator shim; the game exports one only if it has its own allocator.
        exported, _ = pe_symbols(std_runtime[0])
        names = {kind: next(n for n in exported if n.endswith(f"___rust_{kind}")) for kind in ("alloc", "dealloc")}
        try:
            # Every image of the route, the wasm runtime DLL included: none may export or import a second allocator.
            windows_allocator([shipped_exe, *deps.glob("*.dll"), *std_runtime], game, shipped / "allocator-imports.json")
        except ValueError as error:
            failures.append(f"shipped route: {error}")
        print("shipped route allocator imports and exports:\n" + (shipped / "allocator-imports.json").read_text())
        # 3. At run time, which heap each allocator draws from (classified, never freed across).
        heap_exe = shipped / "route_heap.exe"
        run([*route, "--crate-name=route_heap", *extern("horizon_client"), *extern("mod_api"),
             *(flag for arg in identity["link_args"] for flag in ("-C", f"link-arg={arg}")),
             str(FIXTURES / "route_heap.rs"), "-o", str(heap_exe)])
        heap = subprocess.run([str(heap_exe), game.name, std_runtime[0].name, *names.values()],
                              env=runtime_env(shipped), capture_output=True, text=True)
        (shipped / "route-heap.txt").write_text(heap.stdout + heap.stderr)
        print(f"shipped route heaps (exit {heap.returncode}):\n{heap.stdout}{heap.stderr}")
        if heap.returncode != 0 or "ONE ALLOCATOR" not in heap.stdout:
            failures.append("the shipped executable and the game allocate from different heaps (shipped/route-heap.txt)")

    common = ["rustc", "--edition=2024", "-C", "prefer-dynamic", "-L", f"dependency={deps}",
              "-L", f"dependency={sdk / 'host'}", "-L", f"native={sdk / 'native'}", "-L", f"dependency={output}"]
    for crate, source in [("probe_shared", "shared.rs"), ("probe_a", "mod.rs"), ("probe_b", "mod.rs")]:
        extra = [] if crate == "probe_shared" else ["--extern", f"probe_shared={output / (prefix + 'probe_shared' + extension)}"]
        run([*common, "--crate-type=dylib", "--crate-name", crate, *extern("horizon_client"), *extern("mod_api"),
             *extra, str(FIXTURES / source), "-o", str(output / (prefix + crate + extension))])
    executable = output / ("probe.exe" if windows else "probe")
    run([*common, "--crate-name=probe", *extern("horizon_client"), *extern("mod_api"),
         *extern("libloading"), "--extern", f"probe_shared={output / (prefix + 'probe_shared' + extension)}",
         str(FIXTURES / "main.rs"), "-o", str(executable)])
    env = runtime_env(output)
    # The std runtime library is part of the process too and carries its own default-allocator shim.
    images = [executable, *output.glob("*" + extension), *deps.glob("*" + extension), *std_runtime]
    probe_args = [str(executable), str(output / (prefix + "probe_a" + extension)), str(output / (prefix + "probe_b" + extension))]
    if windows:
        try:
            windows_allocator(images, game, output / "allocator-symbols.json")
        except ValueError as error:
            failures.append(str(error))
        print("probe allocator exports/imports:\n" + (output / "allocator-symbols.json").read_text())
        # Run the state probe regardless, for its evidence; its cross-image frees are unsafe if allocators differ.
        state = subprocess.run(probe_args, env=env, capture_output=True, text=True)
        (output / "state-probe.txt").write_text(state.stdout + state.stderr)
        print(f"state probe (exit {state.returncode}):\n{state.stdout}{state.stderr}")
        if state.returncode != 0:
            failures.append(f"state probe exited {state.returncode}")
    else:
        for artifact in images:
            run(["readelf", "-d", str(artifact)])
        # ELF lets the first definition in load order interpose every allocator call, std's included:
        # read the dynamic linker's actual bindings rather than the definitions present.
        bindings = output / "ld-bindings"
        run(probe_args, env={**env, "LD_DEBUG": "bindings", "LD_DEBUG_OUTPUT": str(bindings)})
        bound = {}
        for log in output.glob("ld-bindings.*"):
            for line in log.read_text().splitlines():
                if "normal symbol" in line and any(f"___rust_{kind}'" in line for kind in ("alloc", "dealloc", "realloc", "alloc_zeroed")):
                    source, target = line.split("binding file ", 1)[1].split(": normal symbol")[0].split(" to ")
                    bound.setdefault(Path(target.rsplit(" [", 1)[0]).name, set()).add(Path(source.rsplit(" [", 1)[0]).name)
        (output / "allocator-bindings.json").write_text(json.dumps({k: sorted(v) for k, v in bound.items()}, indent=2))
        if set(bound) != {game.name} or not {executable.name, *(p.name for p in std_runtime)} <= bound[game.name]:
            failures.append(f"allocator calls must all bind to the game: {bound}")
    if failures:
        (output / "result.json").write_text(json.dumps({"result": "fail", "failures": failures}, indent=2))
        sys.exit("FAIL:\n- " + "\n- ".join(failures))
    (output / "result.json").write_text(json.dumps({"result": "pass", "artifacts": {
        p.name: sdk_tools.sha256(p) for p in output.iterdir() if p.suffix in (extension, ".exe") or p == executable
    }}, indent=2))
    print(f"PASS; evidence in {output}")


if __name__ == "__main__":
    main()
