#!/usr/bin/env python3
"""Build portable pinned Modrinth packs; game code/assets remain launcher downloads."""

import argparse
import json
from pathlib import Path
import shutil
import zipfile

try:
    from . import lab, runtime
except ImportError:
    import lab
    import runtime

ROOT = lab.ROOT
CORE = ["fabric-api", "sodium", "lithium", "ferrite-core", "immediatelyfast", "entityculling"]
PROFILES = {
    "baseline": {"title": "Vanilla baseline", "mods": ["fabric-api"], "render": 12, "lod": 0, "heap": 6144},
    "core-performance": {"title": "Full-detail performance comparison", "mods": CORE, "render": 12, "lod": 0, "heap": 6144},
    "m2-balanced": {"title": "M2 Pro balanced", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144},
    "adaptive": {"title": "Adaptive terrain experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "adaptive": True},
    "worker-pacing": {"title": "Precise worker pacing experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "pacing": True},
    "adaptive-pacing": {"title": "Adaptive precise pacing experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "adaptive": True, "pacing": True},
    "queue-wait": {"title": "Debounce queue wait experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "queue_wait": True},
    "adaptive-queue": {"title": "Adaptive pacing and queue wait experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "adaptive": True, "pacing": True, "queue_wait": True},
    "dispatch-retry": {"title": "Rejected dispatch retry experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "dispatch_retry": True},
    "tick-dedup": {"title": "Terrain tick membership experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "tick_dedup": True},
    "culling-empty": {"title": "Empty culling whitelist experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "culling_empty": True},
    "quad-reuse": {"title": "Terrain quad reuse experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "quad_reuse": True},
    "terrain-position": {"title": "Relative terrain position experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "terrain_position": True},
    "culling-rays": {"title": "Culling ray scratch experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "culling_rays": True},
    "foreground-first": {"title": "Foreground-first combined experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "adaptive": True, "pacing": True, "queue_wait": True, "dispatch_retry": True, "tick_dedup": True, "target_fps": 120},
    "foreground-vulkan": {"title": "Foreground-first Vulkan experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "adaptive": True, "pacing": True, "queue_wait": True, "dispatch_retry": True, "tick_dedup": True, "target_fps": 120, "backend": "vulkan"},
    "low-memory": {"title": "Low memory", "mods": CORE, "render": 8, "lod": 0, "heap": 3072},
    "far-distance": {"title": "Far distance", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 256, "heap": 8192},
    "cinematic": {"title": "Cinematic (optional)", "mods": CORE + ["distanthorizons", "iris", "complementary-reimagined"], "render": 12, "lod": 128, "heap": 6144},
    "vulkan": {"title": "Vulkan experiment", "mods": CORE + ["distanthorizons"], "render": 12, "lod": 128, "heap": 6144, "backend": "vulkan"},
}


def read_lock():
    return json.loads((ROOT / "packs/mods.lock.json").read_text())


def validate_selection(lock, selection):
    projects = lock["projects"]
    selected_ids = {projects[name]["project_id"] for name in selection}
    selected_versions = {projects[name]["version_id"] for name in selection}
    for name in selection:
        entry = projects[name]
        if lock["minecraft"] not in entry["game_versions"]:
            raise ValueError(f"Wrong game version: {name}")
        if not entry["filename"].endswith(".zip") and "fabric" not in entry["loaders"]:
            raise ValueError(f"Wrong loader: {name}")
        if Path(entry["filename"]).name != entry["filename"] or "\\" in entry["filename"]:
            raise ValueError(f"Unsafe filename: {name}")
        if not entry["url"].startswith("https://cdn.modrinth.com/"):
            raise ValueError(f"Unexpected download host: {name}")
        for dependency in entry["dependencies"]:
            if dependency["dependency_type"] != "required":
                continue
            if dependency.get("version_id"):
                if dependency["version_id"] not in selected_versions:
                    raise ValueError(f"Missing pinned dependency for {name}: {dependency}")
            elif dependency.get("project_id") not in selected_ids:
                raise ValueError(f"Missing dependency for {name}: {dependency}")


def mod_path(entry):
    return ("shaderpacks/" if entry["filename"].endswith(".zip") else "mods/") + entry["filename"]


def options(profile):
    # Simulation distance stays at the vanilla default in every profile.
    values = {"version": read_lock()["options_data_version"], "graphicsPreset": json.dumps("custom"),
              "renderDistance": profile["render"], "simulationDistance": 12, "enableVsync": "false",
              "maxFps": 260, "guiScale": 2, "onboardAccessibility": "false",
              "preferredGraphicsBackend": json.dumps(profile.get("backend", "opengl"))}
    return "".join(f"{key}:{value}\n" for key, value in values.items())


def overrides(profile_name, mod_bytes):
    profile = PROFILES[profile_name]
    files = {f"mods/horizon-{read_lock()['pack_version']}.jar": mod_bytes,
             "config/horizon-profile.txt": (profile_name + "\n").encode(),
             "options.txt": options(profile).encode()}
    experiment = any(profile.get(key, False) for key in ["adaptive", "pacing", "queue_wait", "dispatch_retry", "tick_dedup", "culling_empty", "culling_rays", "terrain_position", "quad_reuse"])
    config = ROOT / "packs/config" / f"{'m2-balanced' if experiment else profile_name}.toml"
    files["config/horizon-adaptive.properties"] = (
        f"enabled={str(profile.get('adaptive', False)).lower()}\ntargetFps={profile.get('target_fps', 60)}\n").encode()
    files["config/horizon-worker-pacing.properties"] = (
        f"enabled={str(profile.get('pacing', False)).lower()}\n").encode()
    files["config/horizon-queue-wait.properties"] = (
        f"enabled={str(profile.get('queue_wait', False)).lower()}\n").encode()
    files["config/horizon-dispatch-retry.properties"] = (
        f"enabled={str(profile.get('dispatch_retry', False)).lower()}\n").encode()
    files["config/horizon-tick-dedup.properties"] = (
        f"enabled={str(profile.get('tick_dedup', False)).lower()}\n").encode()
    files["config/horizon-culling-empty.properties"] = (
        f"enabled={str(profile.get('culling_empty', False)).lower()}\n").encode()
    files["config/horizon-culling-rays.properties"] = (
        f"enabled={str(profile.get('culling_rays', False)).lower()}\n").encode()
    files["config/horizon-quad-reuse.properties"] = (
        f"enabled={str(profile.get('quad_reuse', False)).lower()}\n").encode()
    files["config/horizon-terrain-position.properties"] = (
        f"enabled={str(profile.get('terrain_position', False)).lower()}\n").encode()
    files["config/horizon-terrain-read-wait.properties"] = (
        f"enabled={str(bool(profile['lod'])).lower()}\n").encode()
    if profile["lod"]:
        if not config.exists():
            raise FileNotFoundError(f"Missing verified DH profile config: {config}")
        files["config/DistantHorizons.toml"] = config.read_bytes()
    if profile_name == "cinematic":
        # Start with shaders disabled: users explicitly select their visual/performance budget.
        files["config/iris.properties"] = b"enableShaders=false\nshaderPack=ComplementaryReimagined_r5.9.3.zip\n"
    return files


def write_zip(path, entries):
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name in sorted(entries):
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            mode = 0o100755 if name.endswith(".command") else 0o100644
            info.external_attr = mode << 16
            archive.writestr(info, entries[name])


def build(profile_name, mod_jar):
    lock = read_lock()
    profile = PROFILES[profile_name]
    validate_selection(lock, profile["mods"])
    files = []
    for name in profile["mods"]:
        entry = lock["projects"][name]
        files.append({"path": mod_path(entry), "hashes": entry["hashes"], "downloads": [entry["url"]],
                      "fileSize": entry["size"], "env": {"client": "required", "server": "unsupported"}})
    index = {"formatVersion": 1, "game": "minecraft", "versionId": lock["pack_version"],
             "name": "Horizon — " + profile["title"],
             "summary": "Vanilla mechanics, pinned performance mods, measured frame and tick timings.",
             "files": files, "dependencies": {"minecraft": lock["minecraft"], "fabric-loader": lock["fabric_loader"]}}
    entries = {"modrinth.index.json": json.dumps(index, indent=2).encode()}
    entries.update({"overrides/" + name: data for name, data in overrides(profile_name, mod_jar.read_bytes()).items()})
    output = ROOT / "dist" / f"horizon-{profile_name}-{lock['pack_version']}.mrpack"
    write_zip(output, entries)
    print(f"Built {output.name}: {len(files)} pinned downloads")
    return output


def prepare_instance(profile_name, destination, mod_jar):
    lock = read_lock()
    profile = PROFILES[profile_name]
    validate_selection(lock, profile["mods"])
    destination.mkdir(parents=True, exist_ok=True)
    for name in profile["mods"]:
        entry = lock["projects"][name]
        source = runtime.fetch(entry["url"], ROOT / ".local/pack-cache" / entry["filename"],
                               entry["hashes"]["sha512"], "sha512")
        target = destination / mod_path(entry)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    runtime.install_original_mod(mod_jar, destination / "mods", f"horizon-{lock['pack_version']}.jar")
    for name, data in overrides(profile_name, mod_jar.read_bytes()).items():
        if name.startswith("mods/"):
            continue
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    print(f"Prepared {profile_name} instance: {destination}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=PROFILES)
    parser.add_argument("--instance", type=Path, help="Local demo smoke-test directory (no normal account launch)")
    parser.add_argument("--mod", type=Path, default=ROOT / "dist" / f"horizon-{read_lock()['pack_version']}.jar")
    args = parser.parse_args()
    if not args.mod.exists():
        parser.error("Build the original mod first: python3 tools/build_mod.py")
    if args.instance:
        if not args.profile:
            parser.error("--instance requires --profile")
        prepare_instance(args.profile, args.instance.resolve(), args.mod)
    else:
        for profile in [args.profile] if args.profile else PROFILES:
            build(profile, args.mod)


if __name__ == "__main__":
    main()
