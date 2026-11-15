"""Stage a local trace engine using the game's native entry template; never builds or runs it."""
import json
import os
from pathlib import Path
import shutil
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import lab

fixture = ROOT / 'tools/fixtures/paper-trace-mod'
manifest = tomllib.loads((fixture / 'horizon-mod.toml').read_text())
project = lab.LOCAL / 'paper-parity/engine-project'
(project / 'src').mkdir(parents=True, exist_ok=True)
quote = lambda value: json.dumps(str(value).replace('\\', '/'))
cargo = '[package]\nname="paper-trace-engine"\nversion="0.1.0"\nedition="2024"\n[workspace]\n'
cargo += '[profile.release]\nlto=false\ncodegen-units=16\n[dependencies]\n'
for name, path in [('horizon-client', ROOT / 'crates/horizon-client'),
                   ('horizon-mod-api', ROOT / 'crates/horizon-mod-api'),
                   ('horizon-mod-package', ROOT / 'crates/horizon-mod-package'), ('paper-trace-mod', fixture)]:
    cargo += f'{name}={{path={quote(path)},default-features=false}}\n'
cargo += '[patch.crates-io]\n'
for name, patch in tomllib.loads((ROOT / 'Cargo.toml').read_text())['patch']['crates-io'].items():
    cargo += f'{name}={{path={quote(ROOT / patch["path"])}}}\n'
(project / 'Cargo.toml').write_text(cargo)
# Match the game's Windows stack allowance for this executable, on GNU and MSVC.
(project / 'build.rs').write_text('fn main() { if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {'
    'let arg = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") '
    '{"/STACK:8388608"} else {"-Wl,--stack,8388608"}; println!("cargo:rustc-link-arg={arg}"); }}')
main = (ROOT / 'crates/horizon-client/src/native_main.rs.template').read_text()
main = main.replace('/* MOD_ENTRIES */', f'{quote(manifest["id"])} => paper_trace_mod::hooks,')
main = main.replace('/* MOD_SLOTS */', '')
main = main.replace('/* BUILT_MODS */', 'mod_api::__private::native::BuiltMod {'
    f'id: {quote(manifest["id"])}, version: {quote(manifest["version"])}, '
    f'side: mod_api::Side::{manifest.get("side", "client").title()} }},')
(project / 'src/main.rs').write_text(main)
target = Path(os.environ['CARGO_TARGET_DIR']) / 'release/horizon-native-mods' / manifest['id']
shutil.copytree(fixture, target, dirs_exist_ok=True, ignore=shutil.ignore_patterns('__pycache__', 'target', 'Cargo.lock'))
print(f'Staged {project}; build it with cargo build --release --manifest-path "{project / "Cargo.toml"}"')
