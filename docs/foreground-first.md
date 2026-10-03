# Combined foreground-first experiments

`foreground-first` combines the adaptive background-terrain budget, precise
worker pacing, bounded/wakeable queue waits and rejected-dispatch retries.
`foreground-vulkan` uses the same combination with Vulkan preference and the
game's OpenGL fallback. Both retain balanced mods, 12 full-detail chunks,
128-chunk LOD radius, two DH workers and simulation distance 12.

The adaptive target is 120 FPS to suit the first M2 Pro/ProMotion testing target.
It is a heuristic frame-time goal, not an FPS cap or a guaranteed rate. The
controller reduces background duty cycle when frames fall behind that target
or heap pressure rises. This can delay distant-terrain completion. It cannot
identify whether GPU, CPU, disk, generation or other work caused a slow frame.
Uncapped game options do not override VSync, driver or display behavior.

Start with balanced, then compare the combined OpenGL profile at equal graphics,
heap and world settings. The individual profiles remain available for isolating
each hook. Compare `foreground-first` against `foreground-vulkan` only after the
isolated Vulkan pack works on the device. Each combined profile explicitly
enables all four controls; changing `/horizon adaptive off` disables only the
adaptive controller. Other hooks require their configuration flag and restart.

Change `targetFps` in `config/horizon-adaptive.properties` to evaluate a different
goal, then restart. The recorder captures the goal, actual applied hook flags,
backend and effective video settings. Java compact headers and collector changes
remain separate optional experiments. Normal simulation/tick rules are retained.

These profiles combine measured scheduler mechanisms, not independently measured
Minecraft FPS gains that can be added together. Their interaction, visible
terrain latency, memory and frame pacing need whole-game testing on real GPUs.
