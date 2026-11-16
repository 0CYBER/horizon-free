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
