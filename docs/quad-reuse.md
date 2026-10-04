# Reusing discarded terrain quads

A private live allocation recording sampled Distant Horizons 3.3.4's
`LodQuadBuilder.getOrCreateBufferQuad` on terrain render-loader threads. The
builder obtains a temporary quad before trying to merge it with the last face.
After a successful adjacent-face premerge, the incoming temporary quad is
unused. The original method returns without retaining it in its existing pool.

The isolated `quad-reuse` profile keeps at most one such discarded quad per
builder. It supplies that object only at the two fresh-constructor sites in
`getOrCreateBufferQuad`; selection from the publisher's existing pool stays
upstream. A successful merge requires a false error flag on both quads, matching
the constructor's initial error state. The existing setter replaces every other
stored field before use. This preserves the existing pool's ordering and error
flags instead of adding a discarded object to that pool. No geometry list,
merge predicate, sorting, lighting, texture/material identifier, upload budget,
world data or simulation rule is replaced.

The hook requires exactly DH 3.3.4 and validates both constructor and both
premerge call sites. It is off in every other profile, including combined
profiles. Disable `enabled` in `config/horizon-quad-reuse.properties` and restart
to compare with the same profile. Reports capture the actual applied flag.

The applied Fabric fixture generates 55,296 input faces spanning all directions,
opaque/translucent colors, light levels and material/tile identifiers. It checks
all stored quad fields before and after upstream merging, including sort keys
and error flags. A deliberately poisoned existing-pool object checks that its
state stays upstream. Identity checks reject aliases in live geometry lists.
Four concurrent builder workers compare geometry and reject shared live quads.
These are generated geometry checks, not rendered-pixel comparison on each GPU.

Private exploratory loops measured about 145 MB caller allocation upstream and
2 MB with reuse over 3,072,000 faces. The same reduction appeared in the initial
production-hook fixture. CPU observations varied: one private pair favored the
hook, another favored upstream, and the first production pair favored the hook.
Fixed-order sequential observations do not establish a statistical speedup.
The fixture favors merging and does not represent every terrain workload.

The hook adds a reference and can retain one discarded object per builder.
Lower allocation is not a measurement of lower retained heap, process RSS,
unified memory, game FPS or Apple Silicon performance. Live startup, capture,
save/reload and independent platform checks are required before recommending it.

Raw [production-hook results](../benchmarks/quad-reuse-2026-10-04/local-applied-hooks.json)
and [private exploratory observations](../benchmarks/quad-reuse-2026-10-04/exploratory-observations.json)
retain protocol and provenance limits. The current fixture fingerprints include
opaque/translucent list boundaries, face indices and list sizes. A baseline
without DH reaches the menu and exits with code zero.

The independent [Linux CI fixture](../benchmarks/quad-reuse-2026-10-04/linux-applied-hooks.json)
matched the geometry and four-worker fingerprints. Its allocation totals matched
the local reduction, but caller CPU was about 40.4 ms upstream versus 44.1 ms
with reuse, so this check does not support a universal CPU improvement. Windows,
macOS ARM and Linux checks pass, and the Linux production JAR matches the local
binary byte-for-byte.

The [live software-rendered observations](../benchmarks/quad-reuse-2026-10-04/live-observations.json)
record 220 control versus 224 candidate frames over roughly 120 seconds, with
zero exclusions and matching captured settings and first saved cameras. Peak
sampled RSS differed by less than one percent. Preceding render-loader allocation
counter rates were higher with reuse in this pair. Task completion/geometry
volumes and cache/JIT effects are not quantitatively controlled, so neither the
small frame/RSS differences nor these counters establish a useful game
performance or memory improvement. The candidate saved, reloaded, saved again
and exited with code zero; the control saved, closed its terrain databases and
exited with code zero.
Actual JSON/CSV comparison and mobile layout checks pass. The hook remains an
optional experiment, disabled in combined profiles.

The [initial shallow-size observations](../benchmarks/quad-reuse-2026-10-04/shallow-size-observations.json)
measure the actual hooked classes with `Instrumentation.getObjectSize` inside
four separate Fabric demo clients on local Linux x64 Java 25. They use four
adjacent north-facing quads, which premerge to one geometry quad in every mode.

| Compact headers | Hook | Builder bytes | Quad bytes | Pending scratch bytes |
| --- | --- | ---: | ---: | ---: |
| Off | Off | 48 | 56 | 0 |
| Off | On | 48 | 56 | 56 |
| On | Off | 40 | 48 | 0 |
| On | On | 48 | 48 | 48 |

Without compact headers, the extra reference fits the measured builder's existing
alignment padding. With compact headers, the builder itself grows by eight bytes.
In this workload both modes add 56 shallow bytes per builder including the pending
scratch. This excludes existing pools, referenced lists and every other game
object. JVM alignment and compressed-reference settings or other architectures
can change layouts. It is not a deep heap, residency, RSS, unified RAM or Mac
measurement, and does not demonstrate a net game memory benefit.

The reusable probe keeps both test binaries outside playable packs and records
JVM runtime, alignment and compressed-reference settings. With Java 25 and the
usual build prerequisites, reproduce it using:

```sh
python3 tools/build_mod.py
python3 tools/test_quad_sizes.py
```

Linux uses Xvfb by default; `--native-display` uses an existing screen. The output
is gitignored `dist/quad-size-integration.json`; game logs and the test agent/fixture
remain in `.local/`. No account or world is needed for this demo-mode probe.

The [reusable local applied probe](../benchmarks/quad-reuse-2026-10-04/local-shallow-size-applied.json)
reproduces the initial sizes and records eight-byte object alignment and compressed
references. All four processes exit successfully. A separate Java source-launcher
check verifies agent loading from a quoted path containing spaces.
