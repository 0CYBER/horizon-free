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
