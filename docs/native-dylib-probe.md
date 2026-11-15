# Exact-build native Rust dylib probe (arismoko/horizon#797)

This is a developer probe, not a native loader. It does not start gameplay, publish
an SDK, change launcher routing, or claim a stable Rust ABI.

## Repeat

Use the launcher's pinned Rust with Clippy; Linux also needs `readelf` (Windows images are read by the
harness itself):

```sh
CARGO_INCREMENTAL=0 python3 tools/prebuilt_game.py build target/probe-sdk
CARGO_INCREMENTAL=0 python3 tools/native_dylib_probe.py --sdk target/probe-sdk --output target/native-dylib-probe
python3 -m unittest discover -s tests -p test_native_dylib_probe.py
```

Windows uses the same Python arguments, with `CARGO_INCREMENTAL=0` set in the
environment, and requires the production GNU target, not MSVC. Use a fresh output
directory for each run. A failure aborts; there is no source-build fallback.

The SDK is built by the existing staging/release/client route without changes.
Before compiling, the harness checks the entire rustc verbose identity and every
SDK artifact digest, and tests a deliberately mismatched compiler identity. Python
tests also exercise compiler and artifact mismatch refusal. This preflight is a
dev harness check, not an embedded production library descriptor or a sandbox.

Four standalone Rust targets are built using explicit SDK metadata: a shared
library, two independently compiled consumer dylibs, and an executable. The
shared dependency explicitly references the game dylib, so rustc can satisfy the
SDK's metadata-only crates from their actual implementation in the game. No rlibs
are copied into the SDK, and no shared Rust crates are rebuilt. Each target gets
rustc metadata checking and Clippy with warnings denied before linking.

The executable dynamically loads both consumers, retaining their handles until
process exit. Assertions check:

- The shared dependency's static address and counter, with ordered mutations
  through both separately loaded consumers.
- Actual `mod_points` registry address identity and cross-image/thread writes.
- Actual `mod_api` crash detail registry writes from the shared dependency, read
  by the executable.
- Actual `mod_api` native-call TLS, entered by the executable and observed by
  consumers; entered by each consumer and dropped by the executable; isolated
  from another thread while sharing the global points registry.
- Heap vectors allocated and dropped across executable, dependency and consumer
  boundaries.
- One allocator. The std runtime library carries its own default-allocator shim,
  so the definitions present prove nothing. On Linux the dynamic linker's actual
  bindings (`LD_DEBUG=bindings`) must send every allocator call, std's own
  included, to the game. Windows binds each import to the DLL it names, with no
  interposition: exactly one image may export the allocator, and every image
  calling it must import it from that one (arismoko/horizon#2394). The shipped route's
  executable is checked the same way, and at run time every allocator in it must
  draw from one heap. The counting-allocator experiment records on Windows why:
  a `#[global_allocator]` in a dylib beside std's DLL splits from it.

`commands.log` records compiler commands, checks, Clippy, dynamic dependency
inspection and runtime assertions. Other evidence includes full `rustc.txt`, the
SDK identity, allocator bindings (Linux) or PE allocator exports/imports
(Windows) and final probe artifact hashes. These
checks would fail if the tested registry, TLS or dependency state had duplicate
copies. They do not instrument every static in every transitive crate: the
remaining sharing evidence is the compiler's exact metadata dependency graph and
actual linked image imports, not a universal state census.

## Linux result

Passed on x86_64-unknown-linux-gnu at SDK source commit
`ff35cdee9511e779c03cae92ae7059e9306b9f16`, with:

```text
rustc 1.99.1 (58cf8f9c3 2026-10-13)
binary: rustc
commit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67
commit-date: 2026-10-13
host: x86_64-unknown-linux-gnu
release: 1.99.1
LLVM version: 23.1.1
```

The unchanged SDK completed release/client compilation (18 existing staging
visibility warnings). All four probe targets passed metadata checking and Clippy;
runtime assertions passed and both Python refusal tests passed. Every
allocator binding in the process, from the executable, both consumers, the shared
dependency, the wasm runtime library and std itself, resolves to the game.
The consumer libraries also allocate vectors and return them for executable drop.

An initial harness draft failed because it omitted the explicit game dependency
from the shared fixture: rustc correctly demanded unavailable rlibs. Adding the
required game reference and SDK dylib extern fixed the harness; no SDK, profile,
crate type or dependency architecture was changed.

Windows GNU runs in `.github/workflows/native-dylib-probe.yml`; its result is
recorded from that job's log, not here. Gameplay,
launcher readiness, library descriptors, malicious/stale mod refusal before OS
loading and performance acceptance are outside this probe's scope.
