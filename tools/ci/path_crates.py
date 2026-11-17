#!/usr/bin/env python3
"""Usage: tools/ci/path_crates.py prepare | save DIR | restore DIR

Carries the build of the path-patched crates (third_party/) through rust-cache. Before it saves, rust-cache deletes
from target/ every package that is neither from a registry or git, nor a workspace member: the crates in this
repository that the workspace excludes. Cargo then rebuilt them on each restore, and a rebuilt dependency makes it
check everything above again (horizon-threads through mimalloc, the renderer through naga and wgpu-hal).

prepare, before main's build: dates those crates' files as tools/ci/pin_mtimes.sh dates a pull request's, and
removes what target/debug still holds of them, so main always builds them anew.
save, after main's build: moves their entries from target/debug into DIR/path-crates, which the cache keeps.
restore, on a pull request after pin_mtimes.sh: moves them back, with their times, unless the pull request changed one
of those crates since the commit in DIR/../sha. Cargo may not look at such a crate's files: a build script that names
its inputs (libmimalloc-sys's, through cc's rerun-if-env-changed) reruns only when those change, so a changed C file
would keep main's build.

Cargo keeps the newest file time, and that file's name, of a crate whose build script names no inputs (naga,
wgpu-hal), and rebuilds the crate when either differs. With every file at 2001-01-01 the name was whichever the
folder listed first, which differs between runners. So prepare and restore date each crate's Cargo.toml one second
later, unless the pull request changed it.
Each does nothing when there is nothing to do, and always exits 0: the worst case is the rebuild.
"""
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

PARTS = (".fingerprint", "build", "deps")
LIBRARIES = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
HASHED = re.compile(r"(.+)-[0-9a-f]{16}(\..+)?")
PINNED = 978307200  # 2001-01-01 00:00:00 UTC, the time pin_mtimes.sh gives unchanged files


def excluded(root):
    """The path packages inside root that are not workspace members: their folders, and their names in target/."""
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked"], cwd=root))
    members = set(metadata["workspace_members"])
    folders, names = [], set()
    for package in metadata["packages"]:
        manifest = Path(package["manifest_path"]).resolve()
        if package["source"] is not None or package["id"] in members or not manifest.is_relative_to(root):
            continue
        folders.append(manifest.parent)
        targets = [target["name"] for target in package["targets"] if LIBRARIES & set(target["kind"])]
        for name in [package["name"], *targets]:
            for spelling in (name, name.replace("-", "_")):
                names.update((spelling, f"lib{spelling}"))
    return folders, names


def built(root, names):
    """The entries of target/debug that belong to the named packages, with the part each is in."""
    for part in PARTS:
        folder = root / "target/debug" / part
        for entry in sorted(folder.iterdir()) if folder.is_dir() else []:
            # build/<package>/<hash> since Cargo 1.100, <name>-<hash> in each part before.
            match = HASHED.fullmatch(entry.name)
            if entry.name in names or match and match.group(1) in names:
                yield part, entry


def move(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists():
        shutil.move(source, destination)


def date_manifests(folders):
    for folder in folders:
        manifest = folder / "Cargo.toml"
        if int(manifest.stat().st_mtime) == PINNED:
            os.utime(manifest, (PINNED + 1, PINNED + 1))


def prepare(root, kept):
    folders, names = excluded(root)
    for folder in folders:
        for path in [folder, *folder.rglob("*")]:
            if not path.is_symlink():
                os.utime(path, (PINNED, PINNED))
    date_manifests(folders)
    removed = 0
    for _, entry in built(root, names):
        shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
        removed += 1
    return removed


def save(root, kept):
    shutil.rmtree(kept, ignore_errors=True)
    folders, names = excluded(root)
    moved = 0
    for part, entry in list(built(root, names)):
        move(entry, kept / part / entry.name)
        moved += 1
    kept.mkdir(parents=True, exist_ok=True)
    (kept / "folders").write_text("\n".join(folder.relative_to(root).as_posix() for folder in folders))
    return moved


def restore(root, kept):
    if not (kept / "folders").is_file():
        return 0
    folders = (kept / "folders").read_text().splitlines()
    built_at = (kept.parent / "sha").read_text().strip()
    if subprocess.run(["git", "diff", "--quiet", built_at, "--", *folders], cwd=root).returncode:
        return 0
    moved = 0
    for part in PARTS:
        entries = sorted((kept / part).iterdir()) if (kept / part).is_dir() else []
        for entry in entries:
            move(entry, root / "target/debug" / part / entry.name)
            moved += 1
    date_manifests([root / folder for folder in folders])
    return moved


def main(argv):
    actions = {"prepare": prepare, "save": save, "restore": restore}
    if len(argv) < 2 or argv[1] not in actions or len(argv) != (2 if argv[1] == "prepare" else 3):
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        return 2
    root = Path.cwd().resolve()
    kept = root / argv[2] / "path-crates" if len(argv) == 3 else None
    try:
        count = actions[argv[1]](root, kept)
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"path_crates: {argv[1]} failed, the path crates will rebuild: {error}")
        return 0
    print(f"path_crates: {argv[1]} handled {count} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
