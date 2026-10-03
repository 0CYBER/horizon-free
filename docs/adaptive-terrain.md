# Adaptive distant-terrain budget

The optional `adaptive` profile keeps the balanced profile's full-detail distance,
distant radius, worker count and simulation settings. Its original controller changes
only Distant Horizons' background worker duty cycle. Existing stable downloads remain
available while this experiment is validated.

Import adaptive into a separate Prism instance with the same Java, heap, world copy
and graphics settings as balanced. `/horizon adaptive on` and `/horizon adaptive off`
change the policy for the current session. Changes are refused during recording;
`/horizon status` shows the current budget.

The opt-in configuration is `config/horizon-adaptive.properties`:

```properties
enabled=true
targetFps=60
```

The target is clamped to 10–240 FPS and respects a lower configured frame cap.
Every two seconds of focused gameplay, the policy considers mean frame duration,
the fraction of frames exceeding twice the target duration, and current used heap.
Slow frames or heap usage above 85% cut the duty cycle to 65% of its previous
value, with a 0.05 floor or the user's lower ceiling. Three consecutive healthy
windows permit a 0.025 increase. The user-configured DH ratio remains the ceiling.
Menus, pauses and loss of focus release the temporary override and discard the partial
window. Another mod's existing or changed override makes the controller yield.
The controller does not write DH settings to disk.

Frame intervals include presentation and frame pacing. A GPU bottleneck can also
trigger this heuristic even when background CPU work was not responsible. Terrain
generation may take longer. Reducing duty cycle does not free cached terrain or
guarantee lower peak memory. No resolution, visual-quality, worker-count,
simulation-distance or tick-rate change is made.

The pinned DH public `threadRuntimeRatio()` API wraps its live configuration entry.
Inspection confirmed that its executor reads the effective entry after each task
and sleeps according to that ratio. Changes affect subsequent pacing without
restarting the pool; individual long-running tasks cannot be preempted. API code
stays in the upstream dependency. Only original `dev.horizonlab` classes are packaged.

Compare **balanced against adaptive**, first with matched cold terrain exploration
and then matched warm terrain. Record three alternating runs per condition. Check
average FPS, slowest-1% FPS, p99 frame duration, integrated-server tick times,
sampled peak heap and process mean CPU cores. Also judge terrain completion time:
improved pacing at the expense of excessive generation delay is a tradeoff to measure.
The local software renderer validates integration only; the M2 Pro must provide
the actual performance result.

JVM heap/nonheap samples are taken once per second during recorded gameplay, plus
start/end samples. Short spikes can be missed. Process CPU time covers the whole
recording, including menus, all Java threads and the integrated server; one mean CPU
core means one CPU-second per elapsed second. JVM memory excludes some native
allocations and GPU memory and is not process RSS.
