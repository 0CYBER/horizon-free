#!/usr/bin/env python3
"""Run the Wire depth regression suite in a real 256 KiB-stack wasm guest.

Requires Python wasmtime (``python -m pip install wasmtime==39.0.0``) and the
wasm32-unknown-unknown Rust target. Run from anywhere with this script. Build
artifacts stay in target/wire-depth-wasm. No game data is needed.

The native tests use private vocabulary constructors and reader state. Compile
an isolated copy of the vocabulary with those same assertions as wasm exports,
without exposing a fixture feature or changing the public mod API. Only the
native thread wrapper is removed: the linker sets the actual guest stack size.
"""
import os
from pathlib import Path
import re
import shutil
import subprocess


def main():
    import wasmtime

    root = Path(__file__).resolve().parents[1]
    work = root / "target/wire-depth-wasm"
    package = work / "vocab"
    source = root / "crates/horizon-mod-vocab"
    shutil.copytree(source / "src", package / "src", dirs_exist_ok=True)
    build = (source / "build.rs").read_text()
    build = build.replace('std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..")',
                          f'std::path::PathBuf::from("{root}")')
    # Generated modules contain absolute paths. Redirect only this crate's
    # modules to the private copy; all generator inputs remain the real catalog.
    build = build.replace('        .expect("generating the mod API");',
        '        .expect("generating the mod API");\n'
        '    for entry in std::fs::read_dir(&out_dir).unwrap() {\n'
        '        let path = entry.unwrap().path();\n'
        '        if path.extension().is_some_and(|ext| ext == "rs") {\n'
        '            let text = std::fs::read_to_string(&path).unwrap();\n'
        f'            std::fs::write(&path, text.replace("{source}", "{package}")).unwrap();\n'
        '        }\n    }')
    (package / "build.rs").write_text(build)
    manifest = (source / "Cargo.toml").read_text()
    manifest = manifest.replace('edition.workspace = true', 'edition = "2024"')
    manifest = manifest.replace('license.workspace = true', 'license = "MIT"')
    manifest = manifest.replace('[lints]\nworkspace = true', '[workspace]')
    manifest = manifest.replace('[lib]', '[lib]\ncrate-type = ["cdylib"]')
    manifest = re.sub(r'path = "([^"]+)"', lambda m: 'path = "' + str((source / m[1]).resolve()) + '"', manifest)
    # Dev dependencies are unnecessary and would create a second vocabulary copy.
    manifest = re.sub(r'\[dev-dependencies\].*?(?=\[features\])', '', manifest, flags=re.S)
    (package / "Cargo.toml").write_text(manifest)
    shutil.copy2(root / "Cargo.lock", package / "Cargo.lock")
    tests = (source / "src/wire/depth_tests.rs").read_text()
    tests = tests.replace('#[test]\nfn ', '#[unsafe(no_mangle)]\npub extern "C" fn ')
    tests, starts = re.subn(r'    std::thread::Builder::new\(\)\n        \.stack_size\(128 \* 1024\)\n        \.spawn\(\|\| \{', '', tests)
    tests, ends = re.subn(r'        \}\)\n        \.unwrap\(\)\n        \.join\(\)\n        \.unwrap\(\);', '', tests)
    assert starts == ends == 3, 'native test wrappers changed; update the adapter'
    (package / "src/wire/wasm_depth_fixture.rs").write_text(tests)
    with (package / "src/wire/mod.rs").open('a') as out:
        out.write('\n#[allow(unsafe_code)]\nmod wasm_depth_fixture;\n')
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    subprocess.run(['cargo', 'rustc', '--manifest-path', str(package / 'Cargo.toml'),
                    '--lib', '--release', '--target', 'wasm32-unknown-unknown',
                    '--target-dir', str(work / 'build'), '--',
                    '-C', 'link-arg=-zstack-size=262144'], env=env, check=True)
    engine = wasmtime.Engine()
    module = wasmtime.Module.from_file(engine, str(work / 'build/wasm32-unknown-unknown/release/mod_vocab.wasm'))
    assert not module.imports, 'fixture unexpectedly requires host imports'
    cases = re.findall(r'pub extern "C" fn (\w+)\(', tests)
    failures = []
    for case in cases:
        # A trap must not corrupt the next test's shadow stack.
        store = wasmtime.Store(engine)
        instance = wasmtime.Instance(store, module, [])
        try:
            instance.exports(store)[case](store)
        except wasmtime.Trap as error:
            failures.append(case)
            print(f'FAIL {case}: {error}', flush=True)
        else:
            print(f'PASS {case}', flush=True)
    if failures:
        raise SystemExit(f'{len(failures)} wasm depth tests failed')


if __name__ == '__main__':
    main()
