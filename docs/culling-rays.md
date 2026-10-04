# Reuse numeric scratch vectors in visibility tests

A game allocation profile found temporary vectors in the pinned Entity Culling
1.11.2 visibility library. Its visibility check creates a direction vector, then
normalizes it; its ray intersection creates a second vector for reciprocals.
These vectors remain within the calculation and carry only numeric values.

The opt-in hook redirects those two constructors to separate vectors owned by
that culling instance. All three components are reset each time. Upstream
normalization, division, intersection, traversal, caches and decisions stay in
place. No approximation, ray limit, global pool or shared scratch object is added.
The instance already keeps mutable camera, target, hit-block and cache state;
the hook retains that single-caller contract. It does not make an instance safe
for concurrent or reentrant calls. Two retained numeric objects per instance
replace temporary objects on this path.

Use `culling-rays` for an isolated comparison with `m2-balanced`. Graphics, LOD
radius, worker budget and other controller settings match. All other profiles,
including the combined foreground experiments, explicitly keep this hook off.
It only applies to Entity Culling 1.11.2. To disable, set `enabled=false` in
`config/horizon-culling-rays.properties` and restart. Frame reports record the
actual `culling_ray_scratch` state; the CLI and offline viewer disclose changes
in that state.

`python tools/test_culling_rays.py` runs the publisher's actual geometry with the
hook disabled and enabled. It compares boolean and provider-query digests for
12,800 bounding-box checks across eight occluder patterns and two interleaved
instances. Each mode performed the same 105,239 block reads. It also checks
2,197 direct ray cases covering signed zero, tiny and large values, infinities
and NaN, unchanged input vectors, separate instance scratch objects and retained
arithmetic values. The test uses private fixture invokers; its JAR and accessors
never enter a release pack. These cases support this workload's equivalence,
rather than proving every world's rendered pixels or every mod callback.

A local warmed loop of two million visibility checks allocated 24,318,096 bytes
with the original constructors versus 920 with scratch reuse. Single caller CPU
observations were about 167 versus 149 ms. Another unchanged run allocated only
6,259,360 bytes, illustrating how JIT compilation and escape analysis vary during
these loops. No fixed per-check allocation or statistical speedup is inferred.
The independent Linux CI run allocated only 1,040 bytes upstream versus 920
with scratch reuse, and its caller CPU observation was about 102 versus 119 ms.
That run provides no useful allocation reduction and is slower in this loop.
The hook therefore stays isolated and disabled in every recommended profile.
Raw [local](../benchmarks/culling-rays-2026-10-04/local-applied-hooks.json) and
[CI](../benchmarks/culling-rays-2026-10-04/linux-applied-hooks.json) observations
include source and workflow provenance. This is not a whole-game FPS/RAM or Mac
GPU benchmark; repeated real hardware comparisons and live allocation profiles
are required.

A fixed-order live Linux software-rendering pair observed about 28.18 KB/s
allocated by `CullThread` with the hook off and 27.96 KB/s with it on, over
roughly 64-second counter spans. This does not establish a useful reduction.
Allocation samples were dominated by whitelist iteration; the recorded stacks
did not include either ray constructor. The separate `culling-empty` experiment
addresses that observed path. The clean 120-second frame captures produced
599 control and 584 candidate frames, with zero exclusions and matching CSV
counts; captured graphics and worker settings matched except the ray flag.
Both worlds saved and exited without crash reports or remaining game processes.
The outer control launcher returned 143 without exposing the game exit status;
that return is not counted as a verified normal game exit. Selected
[live observations](../benchmarks/culling-rays-2026-10-04/live-observations.json)
retain the recording limits. These are per-thread diagnostic observations, not
retained memory, a statistical comparison, or a Mac result.

The factory hook follows the upstream [Mixin constructor redirect contract](https://github.com/SpongePowered/Mixin/blob/master/src/main/java/org/spongepowered/asm/mixin/injection/Redirect.java).
Compilation uses a checksum-verified publisher JAR; the release mod contains only
original Horizon classes. Profiles without Entity Culling must still load.
