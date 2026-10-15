#!/usr/bin/env python3
"""Copy the pinned version's indexed client assets that are not in the game
jar from your own official launcher install: language files and the language
list (pack.mcmeta), the unifont glyph archives and their font definitions, and
the built-in Programmer Art and High Contrast resource packs. Every object's
SHA-1 is verified. Nothing is downloaded; files go to the private client
resource directory (or --target) and are never committed."""
import argparse
from pathlib import Path
import shutil

import lab


def wanted(name):
    return (name == 'pack.mcmeta'
            or name.startswith('minecraft/lang/')
            or name.startswith('minecraft/font/')
            or name.startswith('minecraft/resourcepacks/'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', type=Path, default=lab.RESOURCES,
                        help='prepared resource directory (default: .local/horizon-client/<pinned version>)')
    parser.add_argument('--minecraft-dir', type=Path,
                        help="official launcher game directory (default: the launcher's own)")
    args = parser.parse_args()
    lock, directory = lab.config()
    manifest = lab.import_manifest(lock, directory, args.minecraft_dir)
    store, index = lab.launcher_assets(manifest, args.minecraft_dir)
    target = args.target
    target.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(index, target / 'asset-index.json')
    objects = lab.json.loads(index.read_text())['objects']
    selected = {name: info for name, info in objects.items() if wanted(name)}
    total = 0
    for name, info in selected.items():
        source = lab.launcher_object(store, info['hash'])
        lab.import_copy(source, target / 'assets' / name, info['hash'])
        total += info['size']
    print(f'Client assets ready: {len(selected)} files, {total / 1e6:.1f} MB')


if __name__ == '__main__':
    main()
