# Avoid allocating iterators for empty culling whitelists

A local game JFR found `HashMap.KeySet` iterator allocation in Entity Culling's
dynamic entity/block-entity whitelist checks on `CullThread`. The pinned 1.11.2
methods create a set iterator before discovering that there are no handlers.

The optional hook returns the JDK's shared empty iterator while the live set is
empty. It checks that set every call; it does not cache a whitelist, remember an
empty state, skip a registered handler, or alter culling decisions. Nonempty
sets use their original iterator, callback order, type guard and short circuit.
No per-call callback object is introduced by the hook.

Use the `culling-empty` profile for an isolated comparison with `m2-balanced`.
Both keep the same graphics, LOD radius, worker budget and other controller
settings. The hook is opt-in, restricted to Entity Culling 1.11.2, and can be
disabled with `enabled=false` in `config/horizon-culling-empty.properties` and a
restart. Frame reports record `empty_culling_whitelist`; the CLI and offline
viewer disclose a difference as an experiment variable.

`python tools/test_culling_empty.py` runs the actual publisher methods inside
Fabric. It checks empty iterator dispatch, registration after initialization,
duplicate registration, real block-entity callback order/short circuit, null and
wrong-type inputs, and returning to an empty state. The menu-only fixture does
not exercise callbacks using a valid world entity; nonempty entity iteration
uses the original implementation. Private instrumentation and fixture JARs
never enter release packs. Full game logs remain private.

A local warmed loop of four million empty checks allocated 128,001,248 bytes
upstream versus 1,248 with the hook. CPU was 41 ms versus 33 ms in that single
observation. These counts isolate this allocation path; JIT escape analysis can
produce different results in other calling contexts. They are not game FPS,
whole-game RAM, statistical speed estimates or Mac GPU results. Repeated matching workloads and real hardware tests remain needed. A fixed-order
Linux software-rendered pair observed about 97,445 versus 35,128 allocated bytes
per second on `CullThread`, over separate 64-second counter spans. The captured
graphics matched and both recordings excluded zero frames, but this one pair is
not a repeatable FPS or retained-RAM result. The candidate also encountered an
intermittent shutdown wait after its world had saved; the control exited normally.
See the [terrain read shutdown check](terrain-read-wait.md).

For local world smoke checks, the baseline and candidate use separate copies of
the same peaceful demo-world snapshot. An earlier capture was interrupted by a
zombie killing the player and is excluded from performance comparisons. Release
packs do not change world difficulty. Frame reports now capture actual world
difficulty at their start/end boundaries so comparison tools can flag a mismatch.

The [sanitized Linux CI result](../benchmarks/culling-empty-2026-10-04/linux-applied-hooks.json)
includes its source/workflow provenance and independently checks live registration
and the applied shortcut. CPU timings can include JIT folding of this empty loop;
they are not per-check costs or estimates of game speed.
