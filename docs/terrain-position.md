# Equivalent position arithmetic in the LOD data lookup

A live allocation/CPU investigation sampled Distant Horizons 3.3.4's relative
position conversion on render-loader threads. This is a numeric step within
`FullDataSourceV2.getDataPointAtBlockPos`; the first candidate is a Java hook at
that call, with no native transition, cache, new worker or retained per-source
field. The original library handles containment, column selection and datapoint
search. No writes, world generation, block mechanics or rendering budgets change.

Select `terrain-position` for an isolated comparison with `m2-balanced`.
Graphics, worker and other controller settings match. This hook is off in all
other profiles, including combined profiles, and requires exactly DH 3.3.4.
Disable `enabled` in `config/horizon-terrain-position.properties` and restart.
Reports and both comparison tools capture its actual applied flag.

The conversion retains the original numeric output, including its unusual
negative-coordinate bias. A plain coordinate shift failed a differential test
and was discarded; correcting that apparent rounding is outside this hook's
scope. The replacement computes truncation toward zero for a positive power-of-two
divisor using shifts and sign masks. For a negative coordinate, the publisher's
bias is applied before truncation, and the final relative coordinate retains its
original remainder result. The zero-shift bias is retained too.

The decoded coordinate is a signed 28-bit value. For shift values 0 through 30,
all adjusted negative values and their truncation correction fit a signed 32-bit
integer. Positive coordinates use the same zero correction. Java's masked shift
count is retained for larger valid level differences. A sign-bit divisor (masked
shift 31) and invalid output levels delegate to the original method, preserving
its result or exception class/message. The hook is numeric and has no mutable
shared state.

The actual publisher differential suite checks 1,525,313 cases, including
1,124,986 invalid-level exceptions. Cases include powers-of-two boundaries,
positive/negative coordinates, integer extrema passed through the original
encoding, all output bytes across selected input levels, reserved position bits
and a million randomized position/level combinations. The observed negative
counterexample is retained explicitly.

Private probes rejected a numeric cache, which provided no useful benefit and
was slower with varying inputs. A version with coordinate-sign branches also
became slower with a broad input set and was discarded. A subsequent version
using sign masks matched the randomized results and reduced caller CPU time in
the local synthetic loop across its tested input-set sizes. These are mechanism
observations, not whole-game FPS, retained RAM, GPU cost or Mac performance.
The applied Fabric fixture matched 491,520 public lookups across 30 generated
data owners and six detail levels, with 327,049 populated-column results. Output
and source-data fingerprints matched, and both modes verified whether the
redirect was actually installed.

Timing remains sensitive to warm-up. The first two-million-lookup observation
was about 120 ms caller CPU upstream versus 225 ms with the hook. Subsequent
warmed six-million-lookup sets observed 250/261 ms and, after longer warm-up
and compilation waits, 331/260 ms. The later set favors the hook, but these
are fixed-order single forks with three sequential observations, not independent
repetitions or a statistical speed estimate. Compiler-diagnostic observations
are kept separate. Live captures and independent platform checks are required
before recommending it.
