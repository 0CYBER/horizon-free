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

## Querying the shipped bytecode with REA

`tools/rea_vanilla.py` answers questions about the pinned game JARs with [REA](https://github.com/morluto/rea), which drives a headless JADX over the bytecode itself. Use it next to the Vineflower sources when you need what a text search can't give reliably: every caller of a method, including lambdas and bridge methods; every overload; or a second decompiler's view of a method whose Vineflower output looks wrong.

```sh
python3 tools/rea_vanilla.py classes DispenserBlock
python3 tools/rea_vanilla.py class net.minecraft.world.level.block.DispenserBlock
python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom
python3 tools/rea_vanilla.py callers net.minecraft.server.level.ServerLevel tickChunk
python3 tools/rea_vanilla.py --jar server-game classes DedicatedServer
```

The REA and JADX engine versions are pinned in `research-lock.json`; the first run downloads the engine into `.local/tools/`. REA only admits JADX targets named `.apk`, so the helper keeps an `.apk`-named copy of each JAR in `.local/rea/`. Each query loads the JAR again, which takes about five seconds. Like the Vineflower output, decompiled method bodies are a reading aid: the bytecode and the oracle harnesses remain the behavior reference.
