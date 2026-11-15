# Sync 34 decisions

## Pin and scope

Directly merge upstream `d52edf0341bbe93b551dbb95a7ee9c2b80785a97` onto fork main `20c3cabc384f7846b3f319bbf28454032e4e08c2`. This is the newest successful upstream main push in `ci.yml` at the pin read: [43268730184](https://github.com/horizon-rust/horizon/actions/runs/43268730184). The previous pin is `f24ee1accf1c816736e6e067939d04f3853f474e`.

The parallel contributor sync, fork PR #3575 at `a0858ab2204f61e3c7312667f29be4321691005d`, still integrates the previous f24 pin, not newer upstream source. Its only commits outside merged fork main are its own resolution and main merge. Its branch and PR were inspected read-only and not modified. Recording that head with an ours merge is root's separate operation; that operation changes no source tree and does not require rebasing this sync.

## Complete incoming inventory

| Path | Resolution and behavior |
| --- | --- |
| `crates/horizon-client/src/mobs/ai.rs` | Clean automatic merge. Vex gravity is reset in canonical `finish_tick`, after the living tick, for AI and NoAI routes alike. Registration-time initialization alone would not cover later gravity changes. Both endermite lifetime callers move to the hostile module. |
| `crates/horizon-client/src/mobs/hostile.rs` | Clean automatic merge. `endermite_lifetime` moves unchanged with `pub(super)` visibility: counter increments, persistence and removal threshold remain identical. |
| `crates/horizon-threads/src/stack_dump.rs` | Only conflict: two hunks overlap the fork's already-carried atomic readiness handshake with upstream's equivalent fix. Keep one release store inside the named busy frame, its acquire wait, and upstream's yielding wait rather than the fork's spin hint. No duplicate readiness store remains. Production stack capture is unchanged. |
| `tools/command_exec_corpus.py` | Clean automatic merge. Retain the incoming Vex NoGravity command case and corpus registration, including NoAI and explicitly cleared gravity inputs, a tick, then saved-state queries. It requires the existing real-game/vanilla command harness resources; it was not run locally. |

An independent static mob-area audit found no additional moved-equivalent port necessary. Typed ownership, mod hooks and player-kind boundaries remain unchanged. No retired file, string game-rule API, native diagnostic hook or alternate injection path is restored. No mod API contract or mod wasm runtime changes in this sync.

## Sampling and validation policy

Retain sync 33's independently measured CPU-timer calibration, scaled lower bounds, behavioral ratios and exact live-block checks unchanged. A requested 1 kHz CPU timer need not deliver 1 kHz on tick-accounted kernels; the affected WSL2 kernel has 250 Hz accounting. The resolution follow-up is horizon-rust/horizon#15721. The busy stack readiness handshake is distinct from those sampling tests.

Only touched-file rustfmt and a full workspace/all-target compile-only check are permitted locally. No tests, clippy, parity corpus, harnesses or game runs are executed for this sync. CI supplies runtime evidence.

The separate generated-baseline commit `a98e61e6e0` contains zero caps: no oversized file grew relative to the actual fork base, so the prior sync's three caps are removed and the new upstream marker is recorded. The generator, not a hand edit, produced that result.

Full compile-only validation passed on integrated source `a98e61e6e0`: `CARGO_INCREMENTAL=0 cargo check --workspace --all-targets --keep-going -j 4` completed with zero warnings and zero errors. This includes test-target compilation, not test execution. No runtime result is claimed here.
