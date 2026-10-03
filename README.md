# Minecraft Optimization Lab

A personal research workspace for understanding Minecraft Java Edition and testing original performance improvements, including Rust kernels. Not affiliated with Mojang or Microsoft.

**Current scope:** complete local client/server decompilation for pinned Java Edition **26.3**, class inventories and output checks, and a tested Rust occupancy experiment. This is not a playable Rust port, a rebuilt Minecraft distribution, or proof of a game-level speedup.

The game JARs, libraries, decompiled Java, inventories, and logs live **inside this checkout** under **`.local/`**, which is gitignored. GitHub contains only our tooling, original Rust code, and research documentation. A fresh clone regenerates the local game workspace from official downloads.

## Prepare and inspect the game

Requires Python 3.11+, JDK 25+, and Git. No Microsoft account credentials are needed for these downloads or decompilation. Use your normal licensed launcher and sign-in when playing or collecting client performance measurements.

```sh
git clone https://github.com/ludovic111/minecraft-optimization-lab.git
cd minecraft-optimization-lab
git config core.hooksPath .githooks
python3 tools/lab.py prepare
python3 tools/lab.py decompile
python3 tools/lab.py status
```

`research-lock.json` pins the version manifest and Vineflower 1.12.0 with checksums. Preparation verifies official artifact hashes, extracts the inner server game JAR from its launcher bundle, and supplies client/server libraries to the decompiler. No game process is launched, no game authentication is implemented, and no server `eula.txt` is written by the tooling.

The default decompilation uses four threads and a 6 GiB Java heap, with one side processed at a time:

```sh
python3 tools/lab.py decompile --side client --threads 4 --heap 6G
python3 tools/lab.py decompile --side server --java /path/to/jdk-25/bin/java
```

Outputs:

```text
.local/minecraft/26.3/
  jars/client.jar
  jars/server.jar              # Downloaded launcher bundle
  jars/server-game.jar         # Actual server bytecode
  libraries/{client,server}/
  sources/client/              # Decompiled client, including shared game logic
  sources/server/              # Decompiled dedicated server
  reports/*-inventory.json     # Every class, SourceFile, method count, size
  reports/*-coverage.json      # Missing files and detected failure markers
  reports/*-decompiler.log
```

Inner classes generally share a `.java` file with their enclosing class. Source coverage compares class-file `SourceFile` metadata with emitted Java, and scans Vineflower error markers. An exit code of 2 from `decompile` or `status` means coverage/failure markers need investigation. This does not establish semantic equivalence, preservation of every synthetic method, or recompilability; keep the original bytecode as the reference. Completed source directories are never overwritten automatically. Move a directory aside before repeating decompilation.

For older obfuscated versions, preparation deliberately stops: a version-specific remapping stage is needed. The tool does not silently present obfuscated output as readable source. The current pin can be updated by reviewing an official version manifest and its checksums.

## Run the original Rust experiment

Requires Rust 1.85+; there are no third-party crate dependencies.

```sh
cargo test --workspace
cargo run --release --bin voxel-bench -- 100 > .local/voxel-benchmark.csv
```

`voxel-kernels` compares a scalar reference with a row-bitset algorithm for counting exposed faces in synthetic 16×16×16 sections. It checks empty, full, flat, checkerboard, and seeded random occupancy at three densities. Each result is the median of 21 samples, with warm-up, alternating measurement order, and checks against the reference. The repeat count is configurable.

All cubes are treated as opaque/full, and external neighbors as air. There is no mesh output, texture lookup, AO, lighting, GPU work, cross-section neighborhood, JNI bridge, or Minecraft integration yet. A speedup here applies only to this kernel relative to our scalar reference, not to Minecraft's renderer or its frame rate. See [the optimization plan](docs/optimization-plan.md).

## Develop

```sh
python3 -m unittest discover -s tests -v
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 tools/check_public_tree.py
python3 tools/check_public_tree.py --history
```

CI is offline with respect to Minecraft: it tests the original tooling and Rust kernels without downloading or publishing game files. The optional hooks and CI reject `.local/`, game/profiling binaries, and `.java` files even if force-added. Original Java mod sources would need an explicit reviewed exception before adding them. The path check is a guardrail, not a substitute for reviewing a public diff.

## Ownership and references

The [MIT license](LICENSE) applies only to the original code and documentation tracked in this repository. It does not relicense downloaded game content, local decompiled Java, dependency JARs, or Vineflower. Their respective licenses still apply.

Mojang [announced removal of Java Edition obfuscation](https://www.minecraft.net/en-us/article/removing-obfuscation-in-java-edition) without changing the [Minecraft EULA](https://www.minecraft.net/en-us/eula). Keep game code and modified game builds local; publish original tools and independent optimization work. Decompiler behavior is documented by [Vineflower](https://vineflower.org/usage/).
