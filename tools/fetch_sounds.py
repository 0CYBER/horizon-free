#!/usr/bin/env python3
"""Fetch the pinned version's sound assets (sounds.json and .ogg files) from
Mojang's asset index into the private client resource directory, verifying
every object's SHA-1. The files stay in ignored .local/ and are never committed."""
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


def main():
    manifest = json.loads((LOCAL / 'minecraft/26.3/manifest.json').read_text())
    index_info = manifest['assetIndex']
    target = LOCAL / 'horizon-client/26.3'
    index_path = target / 'asset-index.json'
    if not index_path.exists() or hashlib.sha1(index_path.read_bytes()).hexdigest() != index_info['sha1']:
        data = fetch(index_info['url'])
        if hashlib.sha1(data).hexdigest() != index_info['sha1']:
            raise ValueError('asset index checksum mismatch')
        index_path.write_bytes(data)
    objects = json.loads(index_path.read_text())['objects']
    wanted = {name: info for name, info in objects.items()
              if name == 'minecraft/sounds.json' or name.startswith('minecraft/sounds/')}

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
        for size in pool.map(download, wanted.items()):
            total += size
    print(f'Sound assets ready: {len(wanted)} files, {total / 1e6:.1f} MB downloaded')


if __name__ == '__main__':
    main()
