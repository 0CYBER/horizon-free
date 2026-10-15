#!/usr/bin/env python3
"""Fair Horizon vs vanilla 26.3 benchmark (methodology: docs/benchmark.md).

Runs one game per render distance, each from a fresh world (seed 1234,
creative), through the same steps: start to the title screen, create the
world, wait for the spawn area, then per scene teleport, wait for the chunks,
settle and fly a fixed straight path at constant speed. Every frame time of
every flight is kept and the statistics are computed here, by one function,
for every game. RSS is sampled from outside the process for every game too.

    python3 tools/perf_bench.py --game horizon --label horizon-m4
    python3 tools/perf_bench.py --game vanilla --label vanilla-m4
    python3 tools/perf_bench.py --game vanilla-mods --label mods-m4
    python3 tools/perf_bench.py --table horizon-m4 vanilla-m4 mods-m4

Games:
  horizon       target/release/horizon-client, driven over its control socket
  vanilla       your own 26.3 client.jar + Fabric Loader + a dev-only driver
                mod (mods/horizon/src/bench), built here; nothing of the game
                is redistributed
  vanilla-mods  the same + Sodium, Lithium, FerriteCore and ImmediatelyFast
                (pinned in packs/mods.lock.json)

Results: benchmarks/perf/<label>.json. Games run with no audio, from a
scratch directory under .local/perf-bench.
"""
import argparse
import datetime
import json
import math
import os
import platform
import shutil
import socket
import statistics
import subprocess
import sys
import threading
import time
import zipfile
from pathlib import Path

try:
    from . import lab, runtime
except ImportError:
    import lab
    import runtime

ROOT = lab.ROOT
WORK = ROOT / ".local/perf-bench"
VANILLA = ROOT / ".local/minecraft/26.3"
SEED = 1234
WORLD = "PerfBench"
# (name, from xyz, to xyz, pitch): straight 192-block flights over fixed
# scenery of seed 1234 (the same paths as tools/graphics_bench.py).
SCENES = [
    ("village", (-960.0, 125.0, 80.0), (-960.0, 125.0, 272.0), 35.0),
    ("forest", (-1100.0, 120.0, -560.0), (-1100.0, 120.0, -368.0), 25.0),
    ("ocean", (-300.0, 85.0, -420.0), (-300.0, 85.0, -228.0), 10.0),
    ("mountain", (560.0, 260.0, 860.0), (752.0, 260.0, 860.0), 30.0),
]
# One options.txt for every game: vanilla's own keys, which Horizon reads too.
# The graphics settings are those of vanilla's default preset (Fancy) with the
# preset set to Custom, so the render distance is the one asked for (a preset
# would replace it on load). maxFps 260 is "Unlimited"; inactivityFpsLimit
# minimized turns off the 30 FPS limit both games apply after a minute without
# input. Everything not listed stays at vanilla's default.
OPTIONS = {
    "version": "5023",
    "graphicsPreset": '"custom"',
    "renderDistance": "{distance}",
    "simulationDistance": "12",
    "biomeBlendRadius": "2",
    "prioritizeChunkUpdates": "1",
    "ao": "true",
    "renderClouds": '"true"',
    "cloudRange": "64",
    "particles": "0",
    "mipmapLevels": "4",
    "entityShadows": "true",
    "entityDistanceScaling": "1.0",
    "menuBackgroundBlurriness": "5",
    "cutoutLeaves": "true",
    "improvedTransparency": "false",
    "weatherRadius": "10",
    "maxAnisotropyBit": "1",
    "textureFiltering": "1",
    "maxFps": "260",
    "enableVsync": "false",
    "inactivityFpsLimit": '"minimized"',
    "fov": "0.0",
    "guiScale": "0",
    "fullscreen": "false",
    "pauseOnLostFocus": "false",
    "onboardAccessibility": "false",
    "skipMultiplayerWarning": "true",
    "tutorialStep": "none",
    "narrator": "0",
    "soundCategory_master": "0.0",
    "realmsNotifications": "false",
}
# The official launcher's default JVM arguments for 26.3: what players run.
LAUNCHER_JVM = ["-XX:+UnlockExperimentalVMOptions", "-XX:+UseG1GC", "-XX:G1NewSizePercent=20",
                "-XX:G1ReservePercent=20", "-XX:MaxGCPauseMillis=50", "-XX:G1HeapRegionSize=32M"]
PERF_MODS = ["sodium", "lithium", "ferrite-core", "immediatelyfast"]


def stats(frame_ms):
    """Every number of a flight, from its frame times in milliseconds.

    The x% low is the average frame time of the slowest x% of frames, as
    frames per second (so a single 2-second freeze shows in the 0.1% low of a
    10,000-frame flight)."""
    times = sorted(frame_ms)
    n = len(times)
    if n == 0:
        return {"frames": 0}
    total = sum(times)
    pct = lambda p: times[min(n, max(1, math.ceil(n * p))) - 1]

    def low(share):
        slowest = times[n - max(1, math.ceil(n * share)):]
        return 1000.0 * len(slowest) / sum(slowest)

    buckets = [4.17, 8.33, 16.67, 33.33, 50.0, 100.0]
    hist = {}
    lo = 0.0
    for hi in buckets + [float("inf")]:
        key = f"<{hi:g}ms" if hi != float("inf") else f">={buckets[-1]:g}ms"
        hist[key] = sum(1 for t in times if lo <= t < hi) / n
        lo = hi
    return {
        "frames": n,
        "seconds": total / 1000.0,
        "avg_fps": n * 1000.0 / total,
        "low_1pct_fps": low(0.01),
        "low_01pct_fps": low(0.001),
        "p50_ms": pct(0.5),
        "p95_ms": pct(0.95),
        "p99_ms": pct(0.99),
        "p999_ms": pct(0.999),
        "max_ms": times[-1],
        "histogram": hist,
    }


class Sampler(threading.Thread):
    """Samples a process's resident memory every quarter second (ps, so the
    same reading for every game and OS)."""

    def __init__(self, pid):
        super().__init__(daemon=True)
        self.pid, self.samples, self.stop = pid, [], threading.Event()

    def run(self):
        while not self.stop.is_set():
            out = subprocess.run(["ps", "-o", "rss=", "-p", str(self.pid)], capture_output=True, text=True)
            if out.stdout.strip():
                self.samples.append((time.time(), int(out.stdout.split()[0]) * 1024))
            self.stop.wait(0.25)

    def summary(self, windows):
        """Peak over the whole run; steady = median over the flight windows."""
        if not self.samples:
            return {}
        during = [rss for t, rss in self.samples if any(a <= t <= b for a, b in windows)]
        return {
            "rss_peak_mb": max(rss for _, rss in self.samples) / 2**20,
            "rss_steady_mb": statistics.median(during) / 2**20 if during else None,
            "rss_samples": len(self.samples),
        }


def nvidia_memory_mb(pid):
    """Video memory nvidia-smi attributes to the process, if it can."""
    if not shutil.which("nvidia-smi"):
        return None
    out = subprocess.run(["nvidia-smi", "--query-compute-apps=pid,used_memory", "--format=csv,noheader,nounits"],
                         capture_output=True, text=True)
    for line in out.stdout.splitlines():
        p, _, mem = line.partition(",")
        if p.strip() == str(pid):
            return float(mem)
    return None


def write_options(game_dir, distance):
    game_dir.mkdir(parents=True, exist_ok=True)
    text = "".join(f"{k}:{v.format(distance=distance)}\n" for k, v in OPTIONS.items())
    (game_dir / "options.txt").write_text(text)
    return text


def scene_list(args):
    picked = args.scenes.split(",") if args.scenes else [s[0] for s in SCENES]
    return [s for s in SCENES if s[0] in picked]


def yaw(start, end):
    return math.degrees(math.atan2(-(end[0] - start[0]), end[2] - start[2]))


# Horizon -------------------------------------------------------------------

class Control:
    def __init__(self, path):
        self.path = path

    def __call__(self, line, timeout=600):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(timeout)
            s.connect(str(self.path))
            s.sendall((line + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = s.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        reply = json.loads(data)
        if isinstance(reply, dict) and "error" in reply:
            raise RuntimeError(f"{line}: {reply['error']}")
        return reply


def poll(control, check, limit, interval=0.05):
    """Calls control('status') until check(status); returns (status, time)."""
    deadline = time.time() + limit
    while time.time() < deadline:
        try:
            status = control("status", timeout=10)
            if check(status):
                return status, time.time()
        except (OSError, ValueError, RuntimeError):
            pass
        time.sleep(interval)
    raise TimeoutError(f"not ready after {limit} s")


def wait_loaded(control, near, limit):
    """Waits until the player is spawned (near(position)), nothing is queued and
    the resident and generated chunk counts have not changed for a second, as
    the vanilla driver does; returns the time of the last change.
    (pending_chunks alone misses work the threads have already taken; sections
    are remeshed all the time as blocks change, so meshes are not counted.)"""
    deadline = time.time() + limit
    last, changed = None, time.time()
    while time.time() < deadline:
        try:
            s = control("status", timeout=10)
        except (OSError, ValueError, RuntimeError):
            s = {}
        player = s.get("player")
        if not player or not near(player["position"]) or s.get("pending_chunks") != 0 or s.get("screen") == "title":
            last, changed = None, time.time()
        else:
            counts = (s["chunks"], s["generated_chunks"])
            if counts != last:
                last, changed = counts, time.time()
            elif time.time() - changed >= 1.0:
                return changed
        time.sleep(0.05)
    raise TimeoutError(f"not loaded after {limit} s")


def run_horizon(args, distance):
    game_dir = WORK / "horizon"
    shutil.rmtree(game_dir, ignore_errors=True)
    options = write_options(game_dir, distance)
    sock = Path("/tmp") / f"horizon-bench-{os.getpid()}.sock"
    sock.unlink(missing_ok=True)
    client = Path(args.build).resolve() / "horizon-client"
    command = [str(client), "--resources", str(Path(args.resources).resolve()),
               "--control-socket", str(sock), "--no-pause", "--weather", "clear", "--time", "6000"]
    if args.offscreen:
        command.append("--offscreen")
    env = dict(os.environ, HORIZON_NO_AUDIO="1")
    log = (WORK / f"horizon-rd{distance}.log").open("w")
    started = time.time()
    proc = subprocess.Popen(command, cwd=game_dir, env=env, stdout=log, stderr=subprocess.STDOUT)
    sampler = Sampler(proc.pid)
    sampler.start()
    control = Control(sock)
    result = {"options_txt": options, "scenes": {}}
    windows = []
    try:
        title, at = poll(control, lambda s: s.get("screen") == "title" and s.get("renderer"), 300)
        result["renderer"] = title["renderer"]
        result["startup"] = {"title_s": at - started}
        control(f"open {WORLD} {SEED} creative")
        result["startup"]["world_s"] = wait_loaded(control, lambda p: True, args.timeout) - started
        for name, start, end, pitch in scene_list(args):
            control("time 6000")
            meshed = control("status")["meshed_sections"]
            control("tp {} {} {}".format(*start))
            tp = time.time()
            # A zero-length flight holds the player at the start, flying.
            control("fly {0} {1} {2} {0} {1} {2} 0.1 {3}".format(*start, pitch))
            control(f"look {yaw(start, end):.3f} {pitch}")
            near = lambda p: sum((a - b) ** 2 for a, b in zip(p, start)) < 4.0
            scene = {"load_s": wait_loaded(control, near, args.timeout) - tp}
            # Section meshes built while loading and while flying (Horizon only).
            loaded_meshes = control("status")["meshed_sections"]
            scene["meshes_load"] = loaded_meshes - meshed
            time.sleep(args.settle)
            loaded_meshes = control("status")["meshed_sections"]
            begin = time.time()
            summary = control("fly {} {} {} {} {} {} {} {} raw".format(*start, *end, args.seconds, pitch))
            windows.append((begin, time.time()))
            scene.update(stats(summary.pop("frame_ms")))
            scene["horizon_summary"] = {k: summary.get(k) for k in ("cpu", "gpu", "phases_avg_ms", "slow_phases_avg_ms")}
            status = control("status")
            scene["meshes_flight"] = status["meshed_sections"] - loaded_meshes
            renderer = status.get("renderer") or {}
            scene["gpu_allocated_mb"] = (renderer.get("gpu_allocated_bytes") or 0) / 2**20 or None
            scene["nvidia_mb"] = nvidia_memory_mb(proc.pid)
            result["scenes"][name] = scene
            print_scene(args.game, distance, name, scene)
    finally:
        try:
            control("quit", timeout=60)
        except (OSError, ValueError, RuntimeError):
            pass
        stop(proc)
        sampler.stop.set()
        sock.unlink(missing_ok=True)
    result["memory"] = sampler.summary(windows)
    return result


# Vanilla -------------------------------------------------------------------

def vanilla_manifest():
    if not (VANILLA / "manifest.json").exists():
        sys.exit(f"{VANILLA} missing: prepare Horizon first (tools/prepare_native_client.py)")
    return json.loads((VANILLA / "manifest.json").read_text())


def vanilla_classpath(manifest):
    """The official launcher's client libraries for this OS, from your own prepared copy."""
    jars = [VANILLA / "jars/client.jar"]
    for artifact in lab.client_library_artifacts(manifest):
        jars.append(VANILLA / "libraries/client" / artifact["path"])
    return jars


def extract_natives(jars, target):
    target.mkdir(parents=True, exist_ok=True)
    for jar in jars:
        if "natives-" not in jar.name:
            continue
        with zipfile.ZipFile(jar) as archive:
            for name in archive.namelist():
                if name.endswith((".so", ".dylib", ".jnilib", ".dll")):
                    out = target / Path(name).name
                    if not out.exists():
                        out.write_bytes(archive.read(name))


def build_bench_mod(jars):
    """Compiles the dev-only driver mod against your own client.jar."""
    source = ROOT / "mods/horizon/src/bench"
    build = WORK / "mod"
    classes = build / "classes"
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir(parents=True)
    sources = sorted(map(str, (source / "java").rglob("*.java")))
    subprocess.run(["javac", "--release", "25", "-proc:none", "-encoding", "UTF-8", "-d", str(classes),
                    "-classpath", os.pathsep.join(map(str, jars)), *sources], check=True)
    output = build / "horizon-bench.jar"
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(classes.rglob("*.class")):
            archive.write(path, path.relative_to(classes).as_posix())
        for path in sorted((source / "resources").rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(source / "resources").as_posix())
    return output


def perf_mod_jars():
    """Sodium, Lithium, FerriteCore and ImmediatelyFast for 26.3 (downloaded from
    Modrinth, checked against packs/mods.lock.json, never committed), plus
    Fabric API when one of them needs it."""
    lock = json.loads((ROOT / "packs/mods.lock.json").read_text())
    jars, needs_api = [], False
    for name in PERF_MODS:
        project = lock["projects"][name]
        jar = runtime.fetch(project["url"], ROOT / ".local/pack-cache" / project["filename"],
                            project["hashes"]["sha512"], "sha512")
        with zipfile.ZipFile(jar) as archive:
            depends = json.loads(archive.read("fabric.mod.json")).get("depends", {})
        needs_api |= any(d == "fabric-api" or d.startswith("fabric-") and d != "fabricloader" for d in depends)
        jars.append(jar)
    if needs_api:
        api = lock["projects"]["fabric-api"]
        jars.append(runtime.fetch(api["url"], ROOT / ".local/pack-cache" / api["filename"],
                                  api["hashes"]["sha512"], "sha512"))
    return jars


def run_vanilla(args, distance):
    manifest = vanilla_manifest()
    game_jars = vanilla_classpath(manifest)
    fabric = runtime.fabric_libraries()
    mod = build_bench_mod(game_jars + fabric)
    game_dir = WORK / args.game
    shutil.rmtree(game_dir, ignore_errors=True)
    options = write_options(game_dir, distance)
    mods = game_dir / "mods"
    mods.mkdir()
    shutil.copy(mod, mods)
    loaded_mods = ["horizon-bench"]
    if args.game == "vanilla-mods":
        for jar in perf_mod_jars():
            shutil.copy(jar, mods)
            loaded_mods.append(jar.name)
    natives = WORK / "natives"
    extract_natives(game_jars, natives)
    script = WORK / "script.json"
    script.write_text(json.dumps({
        "world": WORLD, "seed": SEED, "settle": args.settle, "seconds": args.seconds, "timeout": args.timeout,
        "scenes": [{"name": n, "from": a, "to": b, "pitch": p} for n, a, b, p in scene_list(args)],
    }))
    events = WORK / f"{args.game}-rd{distance}.events.jsonl"
    events.unlink(missing_ok=True)
    assets = WORK / "assets"
    (assets / "indexes").mkdir(parents=True, exist_ok=True)
    java = Path(os.environ["JAVA_HOME"]) / "bin/java" if os.environ.get("JAVA_HOME") else "java"
    command = [str(java), f"-Xmx{args.heap}", *LAUNCHER_JVM,
               *runtime.conditional_client_jvm_options(manifest),
               "--enable-native-access=ALL-UNNAMED",
               f"-Djava.library.path={natives}", f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}",
               f"-Dhorizon.bench.script={script}", f"-Dhorizon.bench.out={events}",
               "-Dfabric.log.disableAnsi=true",
               "-cp", os.pathsep.join(map(str, game_jars + fabric)),
               "net.fabricmc.loader.impl.launch.knot.KnotClient",
               "--username", "Bench", "--uuid", "00000000000000000000000000000000", "--accessToken", "0",
               "--offlineDeveloperMode", "--version", "26.3", "--gameDir", str(game_dir),
               "--assetsDir", str(assets), "--assetIndex", manifest["assetIndex"]["id"],
               "--width", str(args.width), "--height", str(args.height)]
    env = dict(os.environ, ALSOFT_DRIVERS="null")
    if sys.platform.startswith("linux"):
        libs = [p for p in (os.environ.get("NIX_LD_LIBRARY_PATH", ""), "/run/opengl-driver/lib") if p]
        env["LD_LIBRARY_PATH"] = ":".join(libs + [env.get("LD_LIBRARY_PATH", "")]).strip(":")
    log = (WORK / f"{args.game}-rd{distance}.log").open("w")
    started = time.time()
    proc = subprocess.Popen(command, cwd=game_dir, env=env, stdout=log, stderr=subprocess.STDOUT)
    sampler = Sampler(proc.pid)
    sampler.start()
    nvidia = None
    try:
        deadline = started + args.timeout * (len(scene_list(args)) + 2)
        while proc.poll() is None and time.time() < deadline:
            time.sleep(0.5)
            if nvidia is None:
                nvidia = nvidia_memory_mb(proc.pid)
    finally:
        stop(proc)
        sampler.stop.set()
    lines = [json.loads(l) for l in events.read_text().splitlines()] if events.exists() else []
    by = lambda name: [e for e in lines if e["event"] == name]
    if not by("done") or by("timeout"):
        raise RuntimeError(f"vanilla run failed: {lines[-1:] if lines else 'no events'}; see {log.name}")
    title, world = by("title")[0], by("world")[0]
    result = {
        "options_txt": options, "mods": loaded_mods, "heap": args.heap,
        "renderer": {k: title[k] for k in ("gpu", "vendor", "backend", "driver", "size",
                                           "render_distance", "simulation_distance")},
        "startup": {"title_s": title["epoch_ms"] / 1000 - started,
                    "world_s": world["ready_epoch_ms"] / 1000 - started},
        "scenes": {},
    }
    windows = []
    loaded = {e["scene"]: e for e in by("loaded")}
    flights = {e["scene"]: e for e in by("fly")}
    for flown in by("flown"):
        name = flown["scene"]
        windows.append((flights[name]["epoch_ms"] / 1000, flown["epoch_ms"] / 1000))
        scene = {"load_s": loaded[name]["load_ms"] / 1000, **stats(flown["frame_ms"]),
                 "chunks": flown["chunks"], "nvidia_mb": nvidia}
        result["scenes"][name] = scene
        print_scene(args.game, distance, name, scene)
    result["memory"] = sampler.summary(windows)
    return result


def stop(proc):
    try:
        proc.wait(timeout=60)
    except subprocess.TimeoutExpired:
        proc.terminate()
        try:
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()


def print_scene(game, distance, name, s):
    print(f"  {game:<12} rd {distance:>2} {name:<9} load {s['load_s']:6.1f} s  {s['avg_fps']:7.1f} fps"
          f"  1% {s['low_1pct_fps']:6.1f}  0.1% {s['low_01pct_fps']:6.1f}"
          f"  p50 {s['p50_ms']:6.2f}  p99 {s['p99_ms']:7.2f} ms", flush=True)


# Report --------------------------------------------------------------------

def machine():
    info = {"host": platform.node(), "os": f"{platform.system()} {platform.release()}",
            "arch": platform.machine(), "python": platform.python_version(), "cpus": os.cpu_count()}
    if sys.platform == "darwin":
        q = lambda key: subprocess.run(["sysctl", "-n", key], capture_output=True, text=True).stdout.strip()
        info["cpu"], info["ram_gb"] = q("machdep.cpu.brand_string"), int(q("hw.memsize") or 0) / 2**30
    else:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                info["cpu"] = line.split(":", 1)[1].strip()
                break
        info["ram_gb"] = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2**30
    return info


def table(labels):
    """Markdown tables comparing saved runs (benchmarks/perf/<label>.json)."""
    runs = [json.loads((ROOT / "benchmarks/perf" / f"{l}.json").read_text()) for l in labels]
    names = [r["label"] for r in runs]
    print("| RD | scene | " + " | ".join(f"{n} avg / 1% / 0.1% low FPS" for n in names) + " |")
    print("| --- | --- |" + " --- |" * len(runs))
    distances = list(runs[0]["distances"])
    for rd in distances:
        for scene in runs[0]["distances"][rd]["scenes"]:
            cells = []
            for r in runs:
                s = r["distances"].get(rd, {}).get("scenes", {}).get(scene)
                cells.append(f"{s['avg_fps']:.0f} / {s['low_1pct_fps']:.0f} / {s['low_01pct_fps']:.0f}" if s else "–")
            print(f"| {rd} | {scene} | " + " | ".join(cells) + " |")
    print()
    print("| RD | " + " | ".join(f"{n}: title / world s, scene load s, RSS steady / peak MB" for n in names) + " |")
    print("| --- |" + " --- |" * len(runs))
    for rd in distances:
        cells = []
        for r in runs:
            d = r["distances"].get(rd)
            if not d:
                cells.append("–")
                continue
            loads = [s["load_s"] for s in d["scenes"].values()]
            m = d.get("memory", {})
            cells.append(f"{d['startup']['title_s']:.1f} / {d['startup']['world_s']:.1f}, "
                         f"{statistics.mean(loads):.1f}, {m.get('rss_steady_mb') or 0:.0f} / {m.get('rss_peak_mb') or 0:.0f}")
        print(f"| {rd} | " + " | ".join(cells) + " |")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--game", choices=["horizon", "vanilla", "vanilla-mods"], default="horizon")
    p.add_argument("--label", help="result name (default <game>-<host>)")
    p.add_argument("--distances", default="8,16,32")
    p.add_argument("--scenes", help="comma-separated subset of " + ",".join(s[0] for s in SCENES))
    p.add_argument("--seconds", type=float, default=12.0, help="flight duration per scene")
    p.add_argument("--settle", type=float, default=2.0, help="seconds held at the start before each flight")
    p.add_argument("--timeout", type=float, default=300.0, help="seconds allowed for each load")
    p.add_argument("--width", type=int, default=1280, help="vanilla window width (Horizon's is 1280x720)")
    p.add_argument("--height", type=int, default=720)
    p.add_argument("--heap", default="2G", help="vanilla -Xmx (the official launcher's default is 2G)")
    p.add_argument("--offscreen", action="store_true",
                   help="Horizon renders into a texture and does not wait for the window (locked macOS sessions)")
    p.add_argument("--resources", default=str(ROOT / ".local/horizon-client/26.3"))
    p.add_argument("--build", default=str(ROOT / "target/release"))
    p.add_argument("--note", help="free text stored with the results")
    p.add_argument("--table", nargs="+", metavar="LABEL", help="print markdown tables of saved runs, and exit")
    args = p.parse_args()
    if args.table:
        table(args.table)
        return
    label = args.label or f"{args.game}-{platform.node().split('.')[0]}"
    WORK.mkdir(parents=True, exist_ok=True)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
                            capture_output=True, text=True).stdout.strip() or None
    report = {
        "label": label, "game": args.game, "date": datetime.date.today().isoformat(), "commit": commit,
        "machine": machine(), "offscreen": args.offscreen, "seed": SEED, "flight_seconds": args.seconds,
        "settle_seconds": args.settle, "note": args.note,
        "scenes": {n: {"from": a, "to": b, "pitch": c} for n, a, b, c in scene_list(args)},
        "distances": {},
    }
    run = run_horizon if args.game == "horizon" else run_vanilla
    for d in [int(x) for x in args.distances.split(",")]:
        load_before = os.getloadavg()
        result = run(args, d)
        result["load_before"], result["load_after"] = load_before, os.getloadavg()
        report["distances"][str(d)] = result
    out = ROOT / "benchmarks/perf" / f"{label}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1) + "\n")
    print(out)


if __name__ == "__main__":
    main()
