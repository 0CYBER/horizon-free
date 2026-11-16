#!/usr/bin/env python3
"""Run the asset-free GPU entity fixture (requires a display and a Vulkan adapter)."""
import argparse
from pathlib import Path
import subprocess
import tempfile

import prebuilt_game
import lab


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--check-only", action="store_true", help="compile the fixture without a display")
    args = parser.parse_args()
    target = prebuilt_game.ROOT / "target" / "standard-pack-gpu-entities"
    target.mkdir(parents=True, exist_ok=True)
    if args.check_only:
        subprocess.run(["cargo", "check", "-p", "horizon-client-render", "--example", "standard_pack_gpu_entities"],
                       cwd=prebuilt_game.ROOT, env=lab.game_env(CARGO_TARGET_DIR=target / "build"), check=True)
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
        subprocess.run(["cargo", "run", "--profile", "dev", "-p", "horizon-client-render",
                        "--example", "standard_pack_gpu_entities", "--", str(output)],
                       cwd=prebuilt_game.ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
