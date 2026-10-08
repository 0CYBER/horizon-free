# World-generation value providers

Horizon implements the scalar distributions needed by the pinned Java Edition
**26.3** cave and canyon definitions. `FloatProvider` supports constant, uniform
and trapezoid distributions. `IntProvider` supports constant, uniform, biased and
very biased distributions, trapezoids, recursively clamped sources and weighted lists.

Decoding retains vanilla numeric narrowing, field defaults and validation rules.
Sampling preserves float operation order, wrapping integer bounds and the number
of random draws. Symmetric integer trapezoids use vanilla's distinct two-draw
subtraction path. Constant float codecs normalize both signed zeros to positive
zero. Invalid random bounds return an error without silently replacing a result.

Run `python3 tools/test_value_provider_parity.py`. The
[local report](../benchmarks/rust-value-provider-2026-10-06/scoped-parity.json)
records **429,920 matching lines per vanilla JAR**, across **504 definitions** and
**5,040 seeded fixtures**. The suite invokes actual provider codecs, reads declared
bounds and compares up to 64 samples per valid fixture. Every sample is followed
by a random long, checking draw consumption as well as output. Both random modes,
signed zeros, overflow, equal/reversed ranges, negative plateaus, malformed fields,
duplicate keys and sampling failures are covered.

Gaussian providers and canonical provider encoding remain pending. A local
Gaussian-math probe found double-bit differences between the host Rust logarithm
and the pinned JVM; using the system logarithm directly would not establish exact
Gaussian parity.
This scoped comparison does not establish complete carver or world-generation
parity. Provider definitions use a bounded decoder; nested integer clamps are
limited to depth 256 and 65,536 total provider nodes.


## Anchored height providers

`HeightProvider` supports constant, uniform, biased, very biased, trapezoid and weighted
heights using all four vertical-anchor kinds. Empty resolved ranges return the
vanilla fallback without drawing randomness. Very biased heights preserve the
guarded inclusive draws, while trapezoid heights retain their distinct sampling
behavior even for symmetric bounds. Anchor resolution and bound arithmetic wrap
as signed Java integers.

Run `python3 tools/test_height_provider_parity.py`. The
[height-provider report](../benchmarks/rust-height-provider-2026-10-06/scoped-parity.json)
records **507,660 matching lines per JAR**. The suite compares actual
`HeightProvider.CODEC` parsing and up to 32 samples per valid definition/context,
with subsequent random longs. It covers all four anchors, ordinary and overflowing
generation contexts, empty/equal ranges, defaults and invalid inner/plateau fields,
and anchor-versus-dispatch preference. Warning logs are excluded. Height-provider encoding remains pending.


## Weighted choices

Weighted integer and height providers retain entry order, nested providers and
zero-weight entries. Integer bounds include zero-weight entries even though they
cannot be selected. Every selection consumes one bounded draw, including a
singleton of weight one. Total weights must be positive and no greater than
2,147,483,647; native constructor rejections are recorded as decode errors.

Rust stores cumulative weights and selects by binary search, avoiding allocation
proportional to total weight. The differential suites cover totals on either side
of vanilla's 64-weight selector threshold, maximum and overflowing totals, nested
choices, malformed entries and exact subsequent random state. Each list is bounded
to 8,192 entries, with a shared 65,536-node budget across nested provider decoding.
Unit tests check those allocation limits and reject oversized branching trees.
