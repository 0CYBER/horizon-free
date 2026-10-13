#!/usr/bin/env python3
"""Prepare an isolated game runtime from your own launcher install; local smoke tests use demo mode."""

from concurrent.futures import ThreadPoolExecutor
import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import zipfile

try:
    from . import lab
except ImportError:
    import lab

ROOT = lab.ROOT
library_allowed = lab.library_allowed
native_library_matches = lab.native_library_matches


def install_original_mod(source, mod_dir, filename=None):
    """Replace our own previous versions, retaining backups outside Fabric's mod scan."""
    mod_dir.mkdir(parents=True, exist_ok=True)
    filename = filename or source.name
    # Retire both current and pre-rebrand filenames to avoid duplicate mod IDs.
    candidates = sorted({*mod_dir.glob("horizon-*.jar"), *mod_dir.glob("horizon-lab-*.jar")})
    for old in candidates:
        if old.name == filename:
            continue
        with zipfile.ZipFile(old) as archive:
            if json.loads(archive.read("fabric.mod.json"))["id"] != "horizonlab":
                raise ValueError(f"Refusing to replace another mod: {old}")
        backup = mod_dir.parent / ".horizon-mod-backups"
        backup.mkdir(exist_ok=True)
        destination = backup / f"{old.stem}-{lab.digest(old, 'sha256')[:12]}.jar"
        shutil.move(str(old), str(destination))
    if source.resolve() != (mod_dir / filename).resolve():
        shutil.copyfile(source, mod_dir / filename)
CACHE = ROOT / ".local" / "runtime"


def jvm_options(heap, compact_headers=False, collector="default"):
    options = ["-Xms512M", f"-Xmx{heap}"]
    if compact_headers:
        options.append("-XX:+UseCompactObjectHeaders")
    if collector != "default":
        options.append({"g1": "-XX:+UseG1GC", "zgc": "-XX:+UseZGC"}[collector])
    return options


def fetch(url, path, checksum, algorithm="sha1"):
    for attempt in range(3):
        try:
            return lab.download(url, path, checksum, algorithm)
        except (OSError, urllib.error.URLError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)


def fabric_libraries():
    profile = json.loads((ROOT / "packs/fabric.lock.json").read_text())
    jars = []
    for library in profile["libraries"]:
        group, artifact, version = library["name"].split(":")
        relative = f"{group.replace('.', '/')}/{artifact}/{version}/{artifact}-{version}.jar"
        jars.append(fetch(library["url"] + relative, CACHE / "fabric" / relative,
                          library["sha256"], "sha256"))
    return jars


def conditional_client_jvm_options(manifest, system=None, architecture=None):
    """Use official OS/architecture flags alongside our managed JVM/classpath options."""
    options = []
    for argument in manifest.get("arguments", {}).get("jvm", []):
        if not isinstance(argument, dict) or not library_allowed(argument, system, architecture):
            continue
        value = argument["value"]
        values = [value] if isinstance(value, str) else value
        if not isinstance(values, list) or any(not isinstance(item, str) or "${" in item for item in values):
            raise ValueError("Unsupported conditional client JVM argument")
        options.extend(values)
    return options


def prepare_client(assets=True):
    """Classpath and natives from the user's own launcher install; nothing of the game is downloaded."""
    lock, directory = lab.config()
    manifest, client = lab.import_client(lock, directory)
    jars = [client]
    natives = CACHE / "natives"
    natives.mkdir(parents=True, exist_ok=True)
    for artifact in lab.client_library_artifacts(manifest):
        jar = lab.import_library(artifact, directory / "libraries/client")
        jars.append(jar)
        with zipfile.ZipFile(jar) as archive:
            for name in archive.namelist():
                if name.endswith((".so", ".dll", ".dylib", ".jnilib")):
                    (natives / Path(name).name).write_bytes(archive.read(name))
    jars.extend(fabric_libraries())
    if assets:
        store, index = lab.launcher_assets(manifest)
        unique = {v["hash"] for v in json.loads(index.read_text())["objects"].values()}
        print(f"Verifying {len(unique)} game assets in {store}...", flush=True)
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(lambda checksum: lab.launcher_object(store, checksum), sorted(unique)))
    return manifest, jars, natives


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "demo"])
    parser.add_argument("--instance", type=Path, default=CACHE / "demo")
    parser.add_argument("--heap", default="4G")
    parser.add_argument("--compact-headers", action="store_true", help="Enable Java 25 compact object headers for this demo process")
    parser.add_argument("--gc", choices=["default", "g1", "zgc"], default="default", help="Select a collector for this process; default leaves JVM policy in use")
    args = parser.parse_args()
    if not re.fullmatch(r"[1-9][0-9]*[MGmg]", args.heap):
        parser.error("--heap must look like 4G")
    manifest, jars, natives = prepare_client()
    if args.command == "prepare":
        return
    instance = args.instance.resolve()
    instance.mkdir(parents=True, exist_ok=True)
    command = ["java", *jvm_options(args.heap, args.compact_headers, args.gc),
               *conditional_client_jvm_options(manifest),
               "--enable-native-access=ALL-UNNAMED",
               f"-Djava.library.path={natives}", f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}",
               "-Dfabric.log.disableAnsi=true", "-cp", os.pathsep.join(map(str, jars)),
               "net.fabricmc.loader.impl.launch.knot.KnotClient",
               "--username", "HorizonDemo", "--version", manifest["id"], "--gameDir", str(instance),
               "--assetsDir", str(lab.minecraft_dir() / "assets"), "--assetIndex", manifest["assetIndex"]["id"],
               "--accessToken", "0", "--uuid", "00000000000000000000000000000000", "--demo",
               "--width", "1280", "--height", "800"]
    subprocess.run(command, cwd=instance, check=True)


if __name__ == "__main__":
    main()
