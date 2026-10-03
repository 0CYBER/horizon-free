# Local decompilation status

Verified on 2026-10-03 using the pinned official Java Edition 26.3 artifacts, Vineflower 1.12.0, and OpenJDK 25.

| Artifact | Input classes | Input methods | Expected Java files | Emitted Java files | Missing files | Files with detected failed-decompilation markers |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Client | 11,383 | 98,739 | 7,301 | 7,301 | 0 | 0 |
| Dedicated server (inner game JAR) | 7,762 | 72,269 | 5,037 | 5,037 | 0 | 0 |

Counts include all class files in each game JAR. Shared code appears in both sides; do not add the class/method counts to estimate unique game code. Nested classes share their enclosing Java source file. All input classes were passed to the decompiler without package filters. Third-party dependency JARs were supplied as references rather than decompiled; runtime/native libraries and the original development/build environment are outside this source recovery.

The full outputs and machine-readable reports are local and gitignored:

```text
.local/minecraft/26.3/sources/client/
.local/minecraft/26.3/sources/server/
.local/minecraft/26.3/reports/client-inventory.json
.local/minecraft/26.3/reports/server-inventory.json
.local/minecraft/26.3/reports/client-coverage.json
.local/minecraft/26.3/reports/server-coverage.json
```

Recheck with `python3 tools/lab.py status`. File coverage and the absence of detected failure markers do not prove every recovered method behaves identically, or that this source can be rebuilt. Original bytecode remains the behavior reference. There has been no gameplay baseline, full Rust port, or measured real-game optimization yet.
