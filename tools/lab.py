#!/usr/bin/env python3
"""Pinned Java Edition acquisition/decompilation. All generated files stay in .local/."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / ".local"
LOCK = ROOT / "research-lock.json"


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def digest(path, algorithm):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, algorithm).hexdigest()


def download(url, destination, checksum, algorithm="sha1"):
    """Never promote an unverified or partial download into the artifact cache."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and digest(destination, algorithm) == checksum:
        return destination
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as stream:
            temporary = Path(stream.name)
            request = urllib.request.Request(url, headers={"User-Agent": "horizon/0.1"})
            with urllib.request.urlopen(request, timeout=120) as response:
                shutil.copyfileobj(response, stream)
        actual = digest(temporary, algorithm)
        if actual != checksum:
            raise ValueError(f"Checksum mismatch for {destination.name}: expected {checksum}, got {actual}")
        temporary.replace(destination)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return destination


def config():
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    version = lock["minecraft"]["version"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", version) or version in {".", ".."}:
        raise ValueError("Unsafe version identifier in lock file")
    return lock, LOCAL / "minecraft" / version


def bundle_entries(archive, name):
    for line in archive.read(name).decode("utf-8").splitlines():
        if not line.strip():
            continue
        checksum, identifier, relative = line.split("\t")
        path = PurePosixPath(relative)
        if path.is_absolute() or ".." in path.parts or "\\" in relative:
            raise ValueError(f"Unsafe bundled path: {relative}")
        yield checksum, identifier, path


def extract_verified(archive, member, destination, checksum):
    data = archive.read(member)
    if hashlib.sha256(data).hexdigest() != checksum:
        raise ValueError(f"Bundled artifact checksum mismatch: {member}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)


def unpack_server(server, directory, version):
    """Modern server downloads are launchers; decompile their inner game JAR."""
    with zipfile.ZipFile(server) as archive:
        if "META-INF/versions.list" not in archive.namelist():
            return server, []
        versions = list(bundle_entries(archive, "META-INF/versions.list"))
        if len(versions) != 1 or versions[0][1] != version:
            raise ValueError("Expected one bundled server matching the pinned version")
        checksum, _, relative = versions[0]
        game = directory / "jars" / "server-game.jar"
        extract_verified(archive, f"META-INF/versions/{relative}", game, checksum)
        libraries = []
        for checksum, _, relative in bundle_entries(archive, "META-INF/libraries.list"):
            destination = directory / "libraries" / "server" / relative
            extract_verified(archive, f"META-INF/libraries/{relative}", destination, checksum)
            libraries.append(destination)
        return game, libraries


class ClassReader:
    """Read SourceFile and method count; no Java text needs to be published."""

    def __init__(self, data):
        self.data = data
        self.offset = 0

    def take(self, count):
        value = self.data[self.offset:self.offset + count]
        if len(value) != count:
            raise ValueError("Truncated class file")
        self.offset += count
        return value

    def number(self, count):
        return int.from_bytes(self.take(count), "big")

    def attributes(self):
        return [(self.number(2), self.take(self.number(4))) for _ in range(self.number(2))]

    def metadata(self):
        if self.take(4) != b"\xca\xfe\xba\xbe":
            raise ValueError("Invalid class file magic")
        self.take(4)  # Minor and major version.
        pool = [None] * self.number(2)
        sizes = {3: 4, 4: 4, 5: 8, 6: 8, 7: 2, 8: 2, 9: 4, 10: 4,
                 11: 4, 12: 4, 15: 3, 16: 2, 17: 4, 18: 4, 19: 2, 20: 2}
        i = 1
        while i < len(pool):
            tag = self.number(1)
            if tag == 1:
                pool[i] = self.take(self.number(2)).decode("utf-8", errors="replace")
            else:
                if tag not in sizes:
                    raise ValueError(f"Unknown constant pool tag: {tag}")
                self.take(sizes[tag])
            i += 2 if tag in {5, 6} else 1
        self.take(6)  # Access, this class, super class.
        self.take(self.number(2) * 2)  # Interfaces.
        for _ in range(self.number(2)):  # Fields.
            self.take(6)
            self.attributes()
        methods = self.number(2)
        for _ in range(methods):
            self.take(6)
            self.attributes()
        source = None
        for name, data in self.attributes():
            if pool[name] == "SourceFile":
                source = pool[struct.unpack(">H", data)[0]]
        return {"source_file": source, "methods": methods}


def inventory(jar):
    classes = []
    packages = Counter()
    with zipfile.ZipFile(jar) as archive:
        for item in archive.infolist():
            if not item.filename.endswith(".class"):
                continue
            metadata = ClassReader(archive.read(item)).metadata()
            path = PurePosixPath(item.filename)
            source = metadata["source_file"] or path.name.split("$")[0].removesuffix(".class") + ".java"
            if PurePosixPath(source).name != source or "\\" in source:
                raise ValueError(f"Unsafe SourceFile attribute in {item.filename}")
            source_path = str(path.parent / source)
            packages[str(path.parent)] += 1
            classes.append({"class": item.filename, "source": source_path,
                            "bytecode_bytes": item.file_size, "methods": metadata["methods"]})
    return {"jar_sha256": digest(jar, "sha256"), "class_count": len(classes),
            "expected_source_files": sorted({c["source"] for c in classes}),
            "method_count": sum(c["methods"] for c in classes),
            "packages": dict(packages.most_common()), "classes": classes}


def prepare():
    lock, directory = config()
    minecraft = lock["minecraft"]
    manifest_path = download(minecraft["manifest_url"], directory / "manifest.json",
                             minecraft["manifest_sha1"])
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest["id"] != minecraft["version"]:
        raise ValueError("Manifest version does not match lock file")
    if "client_mappings" in manifest["downloads"] or "server_mappings" in manifest["downloads"]:
        raise ValueError("This workflow targets unobfuscated releases. Older releases need a remapping stage.")
    if manifest["javaVersion"]["majorVersion"] != minecraft["java_major"]:
        raise ValueError("Manifest Java version does not match lock file")
    jars = {}
    for side in ["client", "server"]:
        artifact = manifest["downloads"][side]
        print(f"Preparing {side} {manifest['id']}...", flush=True)
        jars[side] = download(artifact["url"], directory / "jars" / f"{side}.jar", artifact["sha1"])
    jars["server"], server_libraries = unpack_server(jars["server"], directory, manifest["id"])
    client_artifacts = [library["downloads"]["artifact"] for library in manifest["libraries"]
                        if "artifact" in library.get("downloads", {})]

    def client_library(artifact):
        relative = PurePosixPath(artifact["path"])
        if relative.is_absolute() or ".." in relative.parts or "\\" in artifact["path"]:
            raise ValueError("Unsafe library path")
        return download(artifact["url"], directory / "libraries" / "client" / relative, artifact["sha1"])

    with ThreadPoolExecutor(max_workers=4) as pool:
        client_libraries = list(pool.map(client_library, client_artifacts))
    vf = lock["vineflower"]
    download(vf["url"], LOCAL / "tools" / f"vineflower-{vf['version']}.jar", vf["sha256"], "sha256")
    for side, jar in jars.items():
        data = inventory(jar)
        write_json(directory / "reports" / f"{side}-inventory.json", data)
        print(f"{side}: {data['class_count']} classes, {data['method_count']} methods", flush=True)
    write_json(directory / "prepared.json", {
        "version": manifest["id"], "jars": {k: str(v.relative_to(ROOT)) for k, v in jars.items()},
        "libraries": {"client": [str(p.relative_to(ROOT)) for p in client_libraries],
                      "server": [str(p.relative_to(ROOT)) for p in server_libraries]},
    })
    print(f"Artifacts: {directory}", flush=True)


def coverage(directory, side, source_root=None):
    source_root = source_root or directory / "sources" / side
    data = json.loads((directory / "reports" / f"{side}-inventory.json").read_text(encoding="utf-8"))
    actual = {path.relative_to(source_root).as_posix() for path in source_root.rglob("*.java")}
    expected = set(data["expected_source_files"])
    placeholders = []
    for path in source_root.rglob("*.java"):
        contents = path.read_text(encoding="utf-8", errors="replace")
        if re.search(r"\$VF:.*(?:couldn't|could not|failed)", contents, re.IGNORECASE):
            placeholders.append(path.relative_to(source_root).as_posix())
    return {"side": side, "input_classes": data["class_count"],
            "expected_source_files": len(expected), "emitted_source_files": len(actual),
            "missing_source_files": sorted(expected - actual),
            "files_with_decompiler_error_markers": sorted(placeholders),
            "coverage_note": "Inner classes share their SourceFile; file coverage does not prove method fidelity or recompilability."}


def decompile(args):
    lock, directory = config()
    prepared = json.loads((directory / "prepared.json").read_text(encoding="utf-8"))
    vf = lock["vineflower"]
    tool = LOCAL / "tools" / f"vineflower-{vf['version']}.jar"
    if digest(tool, "sha256") != vf["sha256"]:
        raise ValueError("Decompiler checksum changed; rerun prepare")
    java = shutil.which(args.java)
    if not java:
        raise ValueError("Java not found; install JDK 25 or provide --java /path/to/java")
    version = subprocess.run([java, "-version"], capture_output=True, text=True, check=True)
    match = re.search(r'version "(\d+)', version.stderr + version.stdout)
    if not match or int(match[1]) < lock["minecraft"]["java_major"]:
        raise ValueError(f"JDK {lock['minecraft']['java_major']} or newer is required")
    sides = ["client", "server"] if args.side == "both" else [args.side]
    incomplete = False
    for side in sides:
        destination = directory / "sources" / side
        jar = ROOT / prepared["jars"][side]
        data = json.loads((directory / "reports" / f"{side}-inventory.json").read_text(encoding="utf-8"))
        if digest(jar, "sha256") != data["jar_sha256"]:
            raise ValueError(f"{side} input changed; rerun prepare")
        if destination.exists():
            raise ValueError(f"Output already exists: {destination}. Move it aside before rerunning; it may contain your edits.")
        destination.parent.mkdir(parents=True, exist_ok=True)
        stage = Path(tempfile.mkdtemp(prefix=f"{side}-incomplete-", dir=destination.parent))
        command = [java, f"-Xmx{args.heap}", "-jar", str(tool), "--folder",
                   "--skip-extra-files=1", "--log-level=warn", f"--thread-count={args.threads}"]
        command += [f"--add-external={ROOT / p}" for p in prepared["libraries"][side]]
        command += [str(jar), str(stage)]
        log = directory / "reports" / f"{side}-decompiler.log"
        print(f"Decompiling {side}; log: {log}", flush=True)
        with log.open("w", encoding="utf-8") as stream:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT)
        if result.returncode:
            raise ValueError(f"Decompiler exited {result.returncode}. Partial output retained at {stage}; see {log}")
        stage.rename(destination)
        report = coverage(directory, side)
        report["java_version"] = (version.stderr + version.stdout).strip()
        report["vineflower_version"] = vf["version"]
        report["command"] = command
        write_json(directory / "reports" / f"{side}-coverage.json", report)
        print(json.dumps({k: v for k, v in report.items() if k not in {"command", "java_version"}}, indent=2), flush=True)
        incomplete |= bool(report["missing_source_files"] or report["files_with_decompiler_error_markers"])
    return 2 if incomplete else 0


def status():
    _, directory = config()
    incomplete = False
    for side in ["client", "server"]:
        report = coverage(directory, side)
        print(json.dumps(report, indent=2))
        incomplete |= bool(report["missing_source_files"] or report["files_with_decompiler_error_markers"])
    return 2 if incomplete else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("prepare", help="Download pinned official JARs, libraries, and decompiler; inventory classes")
    decompiler = commands.add_parser("decompile", help="Decompile locally and check source-file coverage")
    decompiler.add_argument("--side", choices=["client", "server", "both"], default="both")
    decompiler.add_argument("--java", default="java")
    decompiler.add_argument("--threads", type=int, default=4)
    decompiler.add_argument("--heap", default="6G")
    commands.add_parser("status", help="Check source coverage and decompiler error markers")
    args = parser.parse_args()
    if getattr(args, "threads", 1) < 1:
        parser.error("--threads must be positive")
    if not re.fullmatch(r"[1-9][0-9]*[MGmg]", getattr(args, "heap", "6G")):
        parser.error("--heap must look like 6G or 512M")
    try:
        if args.command == "prepare":
            prepare()
            return 0
        if args.command == "decompile":
            return decompile(args)
        return status()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
