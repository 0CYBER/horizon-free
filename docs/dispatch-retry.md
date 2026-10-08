# Rejected dispatch retry experiment

A local game recording showed allocation samples repeatedly entering DH's
`ThreadPoolExecutor.AbortPolicy`, exception construction and diagnostic strings.
The direct allocation counters showed substantial allocation on render-loader
workers. This was a scheduler retry path, rather than evidence of useful terrain
generation. Inspecting the pinned DH 3.3.4 dispatcher confirmed that rejection
requeues the same tracked task and immediately tries it again while retaining
the dispatcher lock. A busy or unavailable physical pool can make that loop
allocate and prevent other executors from progressing.

The optional `dispatch-retry` profile uses balanced graphics, mods, distances,
worker count and heap guidance. It enables only the new dispatcher hook; adaptive
budgeting, precise worker pacing and queue waiting stay disabled. Other profiles
explicitly disable the new hook, so existing isolated comparisons remain useful.
It requires exact DH 3.3.4. Set `enabled=false` in
`config/horizon-dispatch-retry.properties` and restart to disable it.

After one rejection for an executor in a dispatch pass, the hook ends that
executor's inner submission loop. Other executors can consume available logical
slots in the same pass. The original catch still requeues the original task or
clears it during shutdown. The original lock, priority ordering, occupied-slot
increments/decrements, exception handling and scheduled-future cancellation
remain in use. A rejected pass requests a later retry after 1 ms; a pass with
only paused executors retains DH's original 1000 ms interval. Shutdown prevents
new retry scheduling. OS timers and lock contention can delay the requested retry.

This bounds repeated rejection within each pass. It still constructs an exception
for a rejected submission, and frequent callers can cause additional passes.
It is not a complete replacement scheduler or proof of faster terrain completion.
No simulation rule, server tick rate, render distance or voxel content is changed.

## Applied-hook integration test

Build with `python tools/build_mod.py`, then run `python tools/test_dispatch.py`.
On Linux the latter uses Xvfb; `--native-display` uses an existing display.
The tool builds a private fixture mod under `.local/mod-build`, runs two isolated
Fabric clients with the dispatcher hook disabled and enabled, and exits before
opening a world. It uses the local demo launcher and requires no account.
Official game libraries/assets and publisher mods remain private downloads.
The fixture never enters a release pack or production JAR.

The fixture creates an actual publisher dispatcher with two executors. It replaces
one executor's physical pool with a one-worker, one-slot pool and deliberately
fills it with blocking work outside the dispatcher. This reproduces rejection
while logical slots remain free. Upstream retains the caller in the retry loop
during a 200 ms observation window and stalls independent queued work. With the
hook, the caller returns and the other executor progresses before releasing the
blocked physical worker. Both runs then require each tracked task to run exactly
once, empty logical queues, zero occupied slots and physical-worker shutdown.

The sanitized output reports the submission caller's wall time, CPU time and
allocated bytes, plus total observed physical-pool rejections. Retry-scheduler
CPU/allocation is excluded from the caller counters. The observation window and
earlier patched return produce different caller durations; these measurements
describe the rejected-submission path, not normal task throughput. Linux CI runs
the applied hooks; Windows and ARM Mac CI compile the fixture and production mod.

In [Linux CI run 37158922187](https://github.com/ludovic111/Horizon/actions/runs/37158922187),
[the raw fixture result](../benchmarks/dispatch-retry-2026-10-04/linux-applied-hooks.json)
reported 216,987,192 caller allocation bytes and 63,790 rejections upstream,
versus 103,384 caller bytes and 6 rejections with the hook. Caller wall times were
401.549 ms and 0.959 ms respectively. These are single, unwarmed failure-path
observations, including initialization/JIT effects, rather than normal throughput
or a fixed-duration whole-process comparison. Both modes completed two tasks.

An initial patched Linux game also loaded, recorded 120 seconds and shut down
normally. Its subsequent JFR capture avoided the earlier allocation storm.
The captures had different cache histories; they do not establish an FPS or
whole-game memory improvement, and this is not an M2 graphics benchmark.
