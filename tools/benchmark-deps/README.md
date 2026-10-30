# Java comparison dependencies

These locks support `tools/perf_bench.py` (vanilla Fabric and performance-mod comparisons)
and `tools/server_bench.py` (Fabric + Lithium). `tools/runtime.py` downloads and verifies
the pinned Fabric libraries. Game files still come from the user's own Minecraft install.

`mods.lock.json` pins Sodium, Lithium, FerriteCore, ImmediatelyFast and Fabric API with
publisher URLs and hashes. `fabric.lock.json` pins the loader and its libraries.
These are developer benchmark inputs.
