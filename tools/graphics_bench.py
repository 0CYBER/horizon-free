#!/usr/bin/env python3
"""Repeatable graphics benchmark for the native client.

Opens a fresh seeded creative world on a virtual screen (vscreen) and flies the
camera along fixed straight paths over a village, a forest, the ocean and a
mountain, at render distances 8, 16 and 32. Each flight replies with frame
times (average FPS, 1% lows, percentiles), the CPU part of each frame and the
GPU time from timestamp queries.

    python3 tools/graphics_bench.py --label baseline
    python3 tools/graphics_bench.py --label baseline --offscreen   # the discrete GPU

`--offscreen` renders into a texture instead of the window, so the desktop's
GPU can be measured even though the headless compositor (vscreen) only
presents software-rendered (lavapipe) frames. Without it, frame times on
vscreen are lavapipe's: use them for CPU-side numbers.
"""
import argparse
import datetime
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEED = "1234"
WORLD = "GraphicsBench"
# (name, from xyz, to xyz, pitch): straight flights at a constant speed over
# fixed scenery of seed 1234. Each path is 192 blocks long.
SCENES = [
    ("village", (-960.0, 125.0, 80.0), (-960.0, 125.0, 272.0), 35.0),
    ("forest", (-1100.0, 120.0, -560.0), (-1100.0, 120.0, -368.0), 25.0),
    ("ocean", (-300.0, 85.0, -420.0), (-300.0, 85.0, -228.0), 10.0),
    ("mountain", (560.0, 260.0, 860.0), (752.0, 260.0, 860.0), 30.0),
]
OPTIONS = """version:4790
enableVsync:false
maxFps:260
renderDistance:{distance}
simulationDistance:12
fov:0.0
guiScale:0
"""


def vscreen_env():
    out = subprocess.run(["vscreen", "env"], cwd=ROOT, capture_output=True, text=True, check=True)
    env = dict(os.environ)
    for part in out.stdout.replace(";", " ").split():
        if "=" in part and not part.startswith("export"):
            k, v = part.split("=", 1)
            env[k] = v
    env.pop("DISPLAY", None)
    return env


class Game:
    def __init__(self, ctl, socket, env, cwd):
        self.ctl, self.socket, self.env, self.cwd = ctl, socket, env, cwd

    def run(self, *words, check=True):
        out = subprocess.run(
            [str(self.ctl), "--socket", str(self.socket), *map(str, words)],
            cwd=self.cwd, env=self.env, capture_output=True, text=True,
        )
        line = out.stdout.strip().splitlines()[-1] if out.stdout.strip() else "{}"
        reply = json.loads(line) if line.startswith("{") else {"error": out.stderr.strip()}
        if check and "error" in reply:
            raise RuntimeError(f"{' '.join(map(str, words))}: {reply} {out.stderr.strip()}")
        return reply


def run_distance(args, ctl, env, cwd, distance):
    (cwd / "options.txt").write_text(OPTIONS.format(distance=distance))
    socket = cwd / "control.sock"
    game = Game(ctl, socket, env, cwd)
    launch = [
        "launch", "--world", WORLD, "--seed", SEED, "--creative",
        "--render-distance", distance, "--resources", args.resources,
        "--no-pause", "--weather", "clear", "--time", "6000",
    ]
    if args.offscreen:
        launch.append("--offscreen")
    game.run(*launch)
    results = {}
    try:
        game.run("wait-ready", 300)
        status = game.run("status")
        results["renderer"] = status.get("renderer")
        for name, start, end, pitch in SCENES:
            if args.scenes and name not in args.scenes.split(","):
                continue
            game.run("tp", *start)
            game.run("look", 0, pitch)
            game.run("time", 6000)
            game.run("wait-ready", 300)
            time.sleep(args.settle)
            summary = game.run("fly", *start, *end, args.seconds, pitch)
            results[name] = summary
            print(
                f"  rd {distance:>2} {name:<9} {summary.get('avg_fps', 0):7.1f} fps"
                f"  1% low {summary.get('low_1pct_fps', 0):7.1f}"
                f"  p99 {summary.get('p99_ms', 0):6.2f} ms"
                f"  cpu {((summary.get('cpu') or {}).get('avg_ms') or 0):6.2f} ms"
                f"  gpu {((summary.get('gpu') or {}).get('avg_ms') or 0):6.2f} ms"
                + "  [" + " ".join(f"{k} {v:.2f}" for k, v in (summary.get("phases_avg_ms") or {}).items()) + "]",
                flush=True,
            )
    finally:
        game.run("quit", check=False)
        for _ in range(100):
            if not socket.exists():
                break
            time.sleep(0.2)
    return results


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--label", default="run")
    p.add_argument("--distances", default="8,16,32")
    p.add_argument("--scenes", help="comma-separated subset of " + ",".join(s[0] for s in SCENES))
    p.add_argument("--seconds", type=float, default=12.0, help="flight duration per scene")
    p.add_argument("--settle", type=float, default=2.0, help="seconds to wait before each flight")
    p.add_argument("--offscreen", action="store_true", help="render offscreen on the discrete GPU")
    p.add_argument("--resources", default=str(ROOT / ".local/horizon-client/26.3"))
    p.add_argument("--build", default=str(ROOT / "target/release"))
    p.add_argument("--out", help="JSON file (default benchmarks/graphics/<label>.json)")
    args = p.parse_args()
    ctl = Path(args.build) / "horizon-ctl"
    if not ctl.exists():
        sys.exit(f"{ctl} missing: cargo build --release -p horizon-client --bins")
    cwd = ROOT / ".local/graphics-bench"
    shutil.rmtree(cwd / "saves" / WORLD, ignore_errors=True)
    cwd.mkdir(parents=True, exist_ok=True)
    env = vscreen_env()
    subprocess.run(["vscreen", "up"], cwd=ROOT, check=True, capture_output=True)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
                            capture_output=True, text=True).stdout.strip()
    report = {
        "label": args.label,
        "date": datetime.date.today().isoformat(),
        "commit": commit,
        "host": os.uname().nodename,
        "offscreen": args.offscreen,
        "seed": SEED,
        "flight_seconds": args.seconds,
        "scenes": {n: {"from": a, "to": b, "pitch": c} for n, a, b, c in SCENES},
        "distances": {},
    }
    for d in [int(x) for x in args.distances.split(",")]:
        report["distances"][str(d)] = run_distance(args, ctl, env, cwd, d)
    out = Path(args.out) if args.out else ROOT / "benchmarks/graphics" / f"{args.label}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + "\n")
    print(out)


if __name__ == "__main__":
    main()
