"""Seed only launcher-owned extras, never another checkout's exported or runtime files."""
import argparse
from pathlib import Path
import shutil

from tools.prepare_output import launcher_extra


def seed(source: Path, target: Path):
    for path in source.rglob("*"):
        relative = path.relative_to(source)
        destination = target / relative
        if not launcher_extra(relative.as_posix()):
            continue
        if path.is_symlink() or not path.is_file():
            continue
        # Never write through a symlink in the target tree. Path.exists() follows the
        # link and is False for a dangling one, so copyfile() would create the file at
        # the link's destination, outside the seeded tree. is_symlink() is an lstat
        # check and sees dangling links too (same as os.path.lexists + islink).
        if destination.is_symlink():
            raise ValueError(f"refusing to seed through symlink destination: {destination}")
        # A symlinked intermediate directory (e.g. target/assets) is just as dangerous:
        # the leaf itself is a regular path, but mkdir/copyfile would still escape the
        # target tree through the link. Check every component between target and the
        # destination's parent, not only the leaf, so no ancestor can redirect a write.
        parent = target
        for component in relative.parts[:-1]:
            parent = parent / component
            if parent.is_symlink():
                raise ValueError(f"refusing to seed through symlink destination: {parent}")
        if destination.exists():
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("target", type=Path)
    args = parser.parse_args()
    seed(args.source, args.target)
