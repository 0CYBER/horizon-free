#!/usr/bin/env python3
"""Fetch the pinned version's indexed client assets that are not in the game
jar: language files and the language list (pack.mcmeta), the unifont glyph
archives and their font definitions, and the built-in Programmer Art and High
Contrast resource packs. Every object's SHA-1 is verified. Files go to the
private client resource directory (or --target) and are never committed."""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
LOCAL = ROOT / '.local'


def fetch(url):
    with urllib.request.urlopen(url, timeout=120) as response:
        return response.read()


def wanted(name):
    return (name == 'pack.mcmeta'
            or name.startswith('minecraft/lang/')
            or name.startswith('minecraft/font/')
            or name.startswith('minecraft/resourcepacks/'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', type=Path, default=LOCAL / 'horizon-client/26.3',
                        help='prepared resource directory (default: .local/horizon-client/26.3)')
    args = parser.parse_args()
    manifest = json.loads((LOCAL / 'minecraft/26.3/manifest.json').read_text())
    index_info = manifest['assetIndex']
    target = args.target
    index_path = target / 'asset-index.json'
    if not index_path.exists() or hashlib.sha1(index_path.read_bytes()).hexdigest() != index_info['sha1']:
        data = fetch(index_info['url'])
        if hashlib.sha1(data).hexdigest() != index_info['sha1']:
            raise ValueError('asset index checksum mismatch')
        target.mkdir(parents=True, exist_ok=True)
        index_path.write_bytes(data)
    objects = json.loads(index_path.read_text())['objects']
    selected = {name: info for name, info in objects.items() if wanted(name)}

    def download(item):
        name, info = item
        path = target / 'assets' / name
        if path.exists() and hashlib.sha1(path.read_bytes()).hexdigest() == info['hash']:
            return 0
        h = info['hash']
        data = fetch(f'https://resources.download.minecraft.net/{h[:2]}/{h}')
        if hashlib.sha1(data).hexdigest() != h:
            raise ValueError(f'checksum mismatch for {name}')
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return len(data)

    total = 0
    with concurrent.futures.ThreadPoolExecutor(16) as pool:
        for size in pool.map(download, selected.items()):
            total += size
    print(f'Client assets ready: {len(selected)} files, {total / 1e6:.1f} MB downloaded')


if __name__ == '__main__':
    main()
