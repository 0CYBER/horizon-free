# Interpreting measured server ticks

Tick execution time omits scheduler sleep. `stats.meanRate` is the inverse average duration, not actual TPS. `observed_ticks_per_second` already counts accepted plus dropped measured invocations divided by wall-clock recording duration. Frozen tick invocations count; paused wall time stays in the denominator. Neither number measures completed simulation work.

Version 0.20 observes the server tick manager after each measured invocation on the server thread. Reports record the first and last sampled configured rate, frozen/stepping/sprinting flags and `runs_game_elements`, plus counters for samples with each nonstandard state. This is read-only instrumentation: it does not set a tick rate, freeze worlds or change game rules. Frame reports do not claim to sample server state; use their accompanying integrated-server tick reports.

The recorder retains which state fields changed between samples. A 20 → 40 → 20 rate change or freeze → unfreeze sequence stays visible even when first and last states match. Both comparison tools mark such recordings as changed workloads, mark differing sampled boundary states, and warn about frozen, stepping, sprinting, suppressed-game-element and nonstandard-rate samples. A consistently configured custom rate can match another report at the same rate, with an explicit warning about that workload. Older, absent or invalid state metadata remains unknown, rather than normal by assumption.

State and tick duration are accepted under the same recording lock, including buffer overflow. The observation path retains scalar values and allocates no maps or lists per invocation; report serialization creates them once. Non-finite or sub-minimum rates are counted as invalid observations without aborting duration capture. The state object is only allocated for tick recordings.

Sampling has limits. These are first/last sampled server states, not exact recording boundary states. Changes entirely between samples can be missed. An integrated server paused by the game menu does not call the measured superclass method, so state counters do not enumerate paused time. The manager flag also does not establish that every entity/chunk was simulated, or detect all other reasons a server skips work. A final stepping tick may already have decremented the remaining-step counter to zero while `runs_game_elements` is still true. Compare world copies, entity populations, completed work and real player workloads separately.

## Validation

Pure recording checks exercise restored intermediate changes, distinct frozen/suppressed states, late observations, absent/invalid manager state and wall-clock TPS calculation under G1, compact headers and ZGC. Offline comparison checks cover map ordering, changed/custom states and legacy/malformed input. The desktop and 390-pixel viewer checks also load the actual discarded paused capture (12 ticks over approximately 120 seconds), display its roughly 0.1 observed TPS and distinguish it from execution capacity. That capture remains excluded from performance evidence.

The private `TickStateIntegration` fixture runs only on a disposable demo-world copy. It starts a short recording, samples normal ticking, changes to 40 configured ticks/second and freezes, steps ten ticks, restores 20/unfrozen state, and requests a five-tick sprint. It checks that sampled boundary states match, all five intermediate state changes/counters survive, and accepted state/duration counts agree. The fixture is compiled in platform CI and excluded from the production JAR and downloadable packs. The [final local applied-hook observation](../benchmarks/tick-state-2026-10-04/live-observations.json) records 285 invocations, with 161 frozen, 9 stepping, 5 sprinting, 151 suppressed-game-element and 161 nonstandard-rate samples. All five change fields survive restored boundary states; CSV count/mean and zero invalid/drop observations are verified, and the client exits with code zero. This is functional evidence rather than a performance comparison. Compilation is not a live Mac result.

Compile the private fixture after building the original mod:

```sh
python3 tools/build_mod.py
python3 -c "from tools.build_mod import build_integration_fixture; build_integration_fixture('tick-state', 'dev.horizonlab.metrics.TickStateIntegration')"
```

Install `.local/mod-build/tick-state-fixture.jar` only into a disposable locally prepared balanced demo instance. Open its copied demo world and dismiss the demo screen. The fixture validates the report and closes the client after its sequence. Never add it to a normal play instance or starter package.
