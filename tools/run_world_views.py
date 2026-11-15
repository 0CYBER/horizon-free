#!/usr/bin/env python3
"""Run the asset-free world views fixture through production source staging (requires a display)."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

import prebuilt_game
import lab


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--bench", action="store_true", help="time frames with and without views (optimized build)")
    parser.add_argument("--check-only", action="store_true", help="compile the staged fixture without a display")
    args = parser.parse_args()
    target = prebuilt_game.ROOT / "target" / "world-views"
    staged = prebuilt_game.stage(target)
    crate = staged / "crates/horizon-client-render"
    shutil.copy2(crate / "fixtures/world_views.rs",
                 crate / "examples/world_views.rs")
    if args.check_only:
        subprocess.run(["cargo", "check", "-p", "horizon-client-render", "--example", "world_views"],
                       cwd=staged, env=lab.game_env(CARGO_TARGET_DIR=target / "build"), check=True)
        return
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="homes-", dir=target) as home:
        homes = {}
        for name in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR",
                     "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            path = Path(home) / name
            path.mkdir(mode=0o700)
            homes[name] = str(path)
        env = lab.game_env(CARGO_TARGET_DIR=target / "build", **homes)
        subprocess.run(["cargo", "run", "--profile", "release" if args.bench else "dev", "-p",
                        "horizon-client-render", "--example", "world_views", "--", str(output),
                        *(["--bench"] if args.bench else [])],
                       cwd=staged, env=env, check=True)


if __name__ == "__main__":
    main()
