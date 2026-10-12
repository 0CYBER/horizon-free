#!/usr/bin/env python3
"""Copy the pinned version's sound assets (sounds.json and .ogg files) from your
own official launcher install into the private client resource directory,
verifying every object's SHA-1. Nothing is downloaded; the files stay in
ignored .local/ and are never committed."""
import argparse
from pathlib import Path
import shutil

import lab


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--minecraft-dir', type=Path,
                        help="official launcher game directory (default: the launcher's own)")
    args = parser.parse_args()
    lock, directory = lab.config()
    manifest = lab.import_manifest(lock, directory, args.minecraft_dir)
    store, index = lab.launcher_assets(manifest, args.minecraft_dir)
    target = lab.LOCAL / 'horizon-client' / manifest['id']
    target.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(index, target / 'asset-index.json')
    objects = lab.json.loads(index.read_text())['objects']
    wanted = {name: info for name, info in objects.items()
              if name == 'minecraft/sounds.json' or name.startswith('minecraft/sounds/')}
    total = 0
    for name, info in wanted.items():
        source = lab.launcher_object(store, info['hash'])
        lab.import_copy(source, target / 'assets' / name, info['hash'])
        total += info['size']
    print(f'Sound assets ready: {len(wanted)} files, {total / 1e6:.1f} MB')


if __name__ == '__main__':
    main()
