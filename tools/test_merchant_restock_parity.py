#!/usr/bin/env python3
"""Exercise real villager AI restock with an open owner merchant menu."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from anvil_dump import plain, region_chunks


def persisted_merchant(root, number):
    matches = []
    for region in root.glob('**/entities/*.mca'):
        for chunk in region_chunks(region).values():
            for entity in plain(chunk, full=True).get('Entities', []):
                if entity.get('UUID') == [113, 1, 0, number]:
                    matches.append(entity)
    if len(matches) != 1:
        raise AssertionError(f'expected one persisted merchant in {root}, got {len(matches)}')
    merchant = matches[0]
    offers = merchant['Offers']['Recipes']
    result = {'id': merchant['id'], 'UUID': merchant['UUID'], 'Xp': merchant['Xp'],
              'RestocksToday': merchant['RestocksToday'],
              'last_restock_positive': merchant['LastRestock'] > 0,
              'offers': [{'uses': o.get('uses', 0), 'demand': o.get('demand', 0),
                          'specialPrice': o.get('specialPrice', 0)} for o in offers]}
    expected = {'id': 'minecraft:villager', 'UUID': [113, 1, 0, number], 'Xp': 20,
                'RestocksToday': 1 if number == 1 else 0, 'last_restock_positive': True,
                'offers': [{'uses': 0, 'demand': 12 if number == 1 else 0, 'specialPrice': -1}]}
    if result != expected:
        raise AssertionError(f'persisted merchant mismatch: {result}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--reference-only', action='store_true')
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    build = args.work_dir.resolve() if args.work_dir else Path(__file__).resolve().parents[1] / '.local/merchant-restock-parity'
    command = [sys.executable, str(Path(__file__).with_name('test_command_exec_parity.py')),
               '--merchant-restock-fixture', '--loaded-fixture', '--work-dir', str(build)]
    if args.binary:
        command += ['--binary', str(args.binary.resolve())]
    if args.reference_only:
        command.append('--reference-only')
    result = subprocess.run(command).returncode
    if result:
        return result
    saved = {'vanilla': [persisted_merchant(build / 'vanilla/universe/harness', n) for n in (1, 2)]}
    if not args.reference_only:
        saved['rust'] = [persisted_merchant(build / 'rust/saves/harness', n) for n in (1, 2)]
    (build / 'persisted-merchants.json').write_text(json.dumps(saved, indent=2) + '\n', encoding='utf-8')
    print('Persisted merchant restock survives menu close and save')
    return 0


if __name__ == '__main__':
    sys.exit(main())
