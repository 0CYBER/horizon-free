#!/usr/bin/env python3
"""Shared Java dependency and platform helpers for Rust comparison benchmarks."""

import json
import time
import urllib.error

try:
    from . import lab
except ImportError:
    import lab

ROOT = lab.ROOT
CACHE = ROOT / ".local" / "runtime"
library_allowed = lab.library_allowed
native_library_matches = lab.native_library_matches


def fetch(url, path, checksum, algorithm="sha1"):
    for attempt in range(3):
        try:
            return lab.download(url, path, checksum, algorithm)
        except (OSError, urllib.error.URLError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)


def fabric_libraries():
    profile = json.loads((ROOT / "tools/benchmark-deps/fabric.lock.json").read_text())
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
