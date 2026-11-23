#!/usr/bin/env python3
"""Compare projectile Entity fields on summon, tick and chunk reload."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from command_exec_corpus import cmd, setup


FIELDS = ('Fire', 'Air', 'HasVisualFire', 'PortalCooldown')
KINDS = ('ender_pearl', 'small_fireball')


def fields(target):
    return [cmd(f'/data get entity {target} {key}') for key in FIELDS]


def groups():
    summon = []
    for kind in KINDS:
        target = f'@e[type={kind},limit=1]'
        for saved in (
            '',
            ',Fire:100s,Air:5s,HasVisualFire:1b,PortalCooldown:100',
            ',Fire:65541,Air:65541,HasVisualFire:257,PortalCooldown:9.75d',
            ',Fire:-4s,Air:-5s,HasVisualFire:0b,PortalCooldown:-3',
            ',Fire:"bad",Air:"bad",HasVisualFire:"bad",PortalCooldown:"bad"',
        ):
            summon.extend([
                cmd(f'/summon {kind} 12.5 234 8.5 {{NoGravity:1b{saved}}}'),
                *fields(target),
                {'op': 'tick', 'n': 3},
                *fields(target),
                *setup(f'/kill {target}'),
            ])

    # A chunk beyond player tickets, with its own forced entity-ticking ticket.
    chunk = '4096 4096'
    wait = {'op': 'wait_chunks', 'chunks': [[256, 256]]}
    reloads = [*setup(f'/forceload add {chunk}'), wait]
    for index, kind in enumerate(KINDS):
        reloads.append(cmd(
            f'/summon {kind} {4100 + 4 * index}.5 300 4104.5 '
            '{NoGravity:1b,Fire:170s,Air:17s,HasVisualFire:1b,PortalCooldown:180}'
        ))
    reloads.append({'op': 'tick', 'n': 3})
    for kind in KINDS:
        reloads.extend(fields(f'@e[type={kind},limit=1]'))
    reloads.extend([
        *setup(f'/forceload remove {chunk}'),
        {'op': 'tick', 'n': 60},
        cmd('/execute unless loaded 4096 300 4096', expect_result=1),
        *(cmd(f'/execute unless entity @e[type={kind}]', expect_result=1) for kind in KINDS),
        *setup(f'/forceload add {chunk}'),
        wait,
    ])
    for kind in KINDS:
        target = f'@e[type={kind},limit=1]'
        reloads.extend([
            cmd(f'/execute if entity {target}', expect_result=1),
            *fields(target),
            *setup(f'/kill {target}'),
        ])
    reloads.extend(setup(f'/forceload remove {chunk}'))

    # Release the ticket before ticking and read the skull as soon as its
    # entity chunk reloads. Its first dry tick must come after that read.
    skull = '@e[type=wither_skull,limit=1]'
    skull_chunk = '4144 4096'
    wait_for_skull = {'op': 'wait_chunks', 'chunks': [[259, 256]]}
    skull_saved = [
        cmd(f'/data get entity {skull} Fire', expect_result=100),
        cmd(f'/data get entity {skull} Air', expect_result=23),
        cmd(f'/data get entity {skull} HasVisualFire', expect_result=1),
        cmd(f'/data get entity {skull} PortalCooldown', expect_result=90),
    ]
    skull_reload = [
        *setup(f'/forceload add {skull_chunk}'),
        wait_for_skull,
        cmd('/summon wither_skull 4152.5 300 4104.5 '
            '{NoGravity:1b,Fire:100s,Air:23s,HasVisualFire:1b,PortalCooldown:90}'),
        *skull_saved,
        *setup(f'/forceload remove {skull_chunk}'),
        {'op': 'tick', 'n': 60},
        cmd('/execute unless loaded 4144 300 4096', expect_result=1),
        cmd(f'/execute unless entity {skull}', expect_result=1),
        *setup(f'/forceload add {skull_chunk}'),
        wait_for_skull,
        cmd(f'/execute if entity {skull}', expect_result=1),
        *skull_saved,
        {'op': 'tick', 'n': 1},
        cmd(f'/data get entity {skull} Fire', expect_result=0),
        cmd(f'/data get entity {skull} Air', expect_result=23),
        cmd(f'/data get entity {skull} HasVisualFire', expect_result=1),
        cmd(f'/data get entity {skull} PortalCooldown', expect_result=89),
        *setup(f'/kill {skull}', f'/forceload remove {skull_chunk}'),
    ]

    block_effects = setup('/setblock 12 233 8 netherrack')
    for kind in (*KINDS, 'wither_skull'):
        target = f'@e[type={kind},limit=1]'
        block_effects.extend([
            *setup('/setblock 12 234 8 fire'),
            cmd(f'/summon {kind} 12.5 234.25 8.5 {{NoGravity:1b,HasVisualFire:1b}}'),
            {'op': 'tick', 'n': 1},
            cmd(f'/data get entity {target} Fire', expect_result=159),
            *setup('/setblock 12 234 8 water'),
            {'op': 'tick', 'n': 1},
            cmd(f'/data get entity {target} Fire', expect_result=20 if kind == 'small_fireball' else 0),
            cmd(f'/data get entity {target} HasVisualFire', expect_result=1),
            *setup(f'/kill {target}'),
        ])

    # Other entities that use the same ThrowableItemProjectile or Fireball codec.
    controls = []
    for kind in ('snowball', 'experience_bottle', 'fireball'):
        target = f'@e[type={kind},limit=1]'
        controls.extend([
            cmd(f'/summon {kind} 12.5 236 8.5 '
                '{NoGravity:1b,Fire:100s,Air:5s,HasVisualFire:1b,PortalCooldown:5}'),
            *fields(target),
            {'op': 'tick', 'n': 1},
            *fields(target),
            *setup(f'/kill {target}'),
        ])
    # These two use the shared portal timer, but have their own fire behavior.
    for kind in ('wind_charge', 'shulker_bullet'):
        target = f'@e[type={kind},limit=1]'
        controls.extend([
            cmd(f'/summon {kind} 12.5 236 8.5 {{NoGravity:1b,PortalCooldown:5}}'),
            cmd(f'/data get entity {target} PortalCooldown', expect_result=5),
            {'op': 'tick', 'n': 1},
            cmd(f'/data get entity {target} PortalCooldown', expect_result=4),
            *setup(f'/kill {target}'),
        ])
    # Rotation on falling blocks, shulkers and clouds: a summoned value is read back, after ticks, and across a chunk save and reload.
    extra_kinds = ('falling_block', 'shulker', 'area_effect_cloud')
    extra = []
    for kind in extra_kinds:
        target = f'@e[type={kind},tag=rot_row,limit=1]'
        for saved in ('', ',Rotation:[90f,10f]', ',Rotation:[400f,100f]', ',Rotation:[0f,0f]',
                      ',Rotation:[270f,370f]', ',Rotation:[-190f,-100f]', ',Rotation:[180f,45f]'):
            extra.extend([
                cmd(f'/summon {kind} 12.5 234 8.5 {{NoGravity:1b,Tags:["rot_row"]{saved}}}'),
                cmd(f'/data get entity {target} Rotation'),
                {'op': 'tick', 'n': 3},
                cmd(f'/data get entity {target} Rotation'),
                *setup(f'/kill {target}'),
            ])
    extra_reload = [*setup(f'/forceload add {chunk}'), wait]
    for index, kind in enumerate(extra_kinds):
        extra_reload.append(cmd(
            f'/summon {kind} {4100 + 4 * index}.5 300 4104.5 {{NoGravity:1b,Tags:["rot_row"],Rotation:[90f,10f]}}'))
    extra_reload.append({'op': 'tick', 'n': 3})
    for kind in extra_kinds:
        extra_reload.append(cmd(f'/data get entity @e[type={kind},tag=rot_row,limit=1] Rotation'))
    extra_reload.extend([
        *setup(f'/forceload remove {chunk}'),
        {'op': 'tick', 'n': 60},
        cmd('/execute unless loaded 4096 300 4096', expect_result=1),
        *setup(f'/forceload add {chunk}'),
        wait,
    ])
    for kind in extra_kinds:
        target = f'@e[type={kind},tag=rot_row,limit=1]'
        extra_reload.extend([
            cmd(f'/execute if entity {target}', expect_result=1),
            cmd(f'/data get entity {target} Rotation'),
            *setup(f'/kill {target}'),
        ])
    extra_reload.extend(setup(f'/forceload remove {chunk}'))
    # cloud fang and eye fire: a summoned value is read back, after ticks, and across a chunk save and reload.
    fire_kinds = ('area_effect_cloud', 'evoker_fangs', 'eye_of_ender')
    fire = []
    for kind in fire_kinds:
        target = f'@e[type={kind},limit=1]'
        for saved in ('', ',Fire:100s', ',Fire:0s', ',Fire:-5s', ',Fire:"bad"'):
            # Fangs and eyes of ender keep a non-numeric Fire as written; vanilla reads it as 0.
            # That is how Horizon stores their loaded tags, not part of this change.
            if saved.endswith('"bad"') and kind != 'area_effect_cloud':
                continue
            fire.extend([
                cmd(f'/summon {kind} 12.5 234 8.5 {{NoGravity:1b{saved}}}'),
                cmd(f'/data get entity {target} Fire'),
                {'op': 'tick', 'n': 3},
                cmd(f'/data get entity {target} Fire'),
                *setup(f'/kill {target}'),
            ])
    fire_reload = [*setup(f'/forceload add {chunk}'), wait]
    for index, kind in enumerate(fire_kinds):
        fire_reload.append(cmd(
            f'/summon {kind} {4100 + 4 * index}.5 300 4104.5 {{NoGravity:1b,Fire:170s}}'))
    fire_reload.append({'op': 'tick', 'n': 3})
    for kind in fire_kinds:
        fire_reload.append(cmd(f'/data get entity @e[type={kind},limit=1] Fire'))
    fire_reload.extend([
        *setup(f'/forceload remove {chunk}'),
        {'op': 'tick', 'n': 60},
        cmd('/execute unless loaded 4096 300 4096', expect_result=1),
        *setup(f'/forceload add {chunk}'),
        wait,
    ])
    for kind in fire_kinds:
        target = f'@e[type={kind},limit=1]'
        fire_reload.extend([
            cmd(f'/execute if entity {target}', expect_result=1),
            cmd(f'/data get entity {target} Fire'),
            *setup(f'/kill {target}'),
        ])
    fire_reload.extend(setup(f'/forceload remove {chunk}'))
    # arrow and orb frozen ticks: a summoned value is read back, after ticks, and across a chunk save and reload.
    frozen_kinds = ('arrow', 'spectral_arrow', 'experience_orb')
    frozen = []
    for kind in frozen_kinds:
        target = f'@e[type={kind},limit=1]'
        for saved in ('', ',TicksFrozen:100', ',TicksFrozen:0', ',TicksFrozen:-5', ',TicksFrozen:"bad"'):
            frozen.extend([
                cmd(f'/summon {kind} 12.5 234 8.5 {{NoGravity:1b{saved}}}'),
                cmd(f'/data get entity {target} TicksFrozen'),
                {'op': 'tick', 'n': 3},
                cmd(f'/data get entity {target} TicksFrozen'),
                *setup(f'/kill {target}'),
            ])
    frozen_reload = [*setup(f'/forceload add {chunk}'), wait]
    for index, kind in enumerate(frozen_kinds):
        frozen_reload.append(cmd(
            f'/summon {kind} {4100 + 4 * index}.5 300 4104.5 {{NoGravity:1b,TicksFrozen:180}}'))
    frozen_reload.append({'op': 'tick', 'n': 3})
    for kind in frozen_kinds:
        frozen_reload.append(cmd(f'/data get entity @e[type={kind},limit=1] TicksFrozen'))
    frozen_reload.extend([
        *setup(f'/forceload remove {chunk}'),
        {'op': 'tick', 'n': 60},
        cmd('/execute unless loaded 4096 300 4096', expect_result=1),
        *setup(f'/forceload add {chunk}'),
        wait,
    ])
    for kind in frozen_kinds:
        target = f'@e[type={kind},limit=1]'
        frozen_reload.extend([
            cmd(f'/execute if entity {target}', expect_result=1),
            cmd(f'/data get entity {target} TicksFrozen'),
            *setup(f'/kill {target}'),
        ])
    frozen_reload.extend(setup(f'/forceload remove {chunk}'))
    # read back, ticks down, and survives a chunk save and reload.
    portal_kinds = ('arrow', 'spectral_arrow', 'experience_orb')
    portal = []
    for kind in portal_kinds:
        target = f'@e[type={kind},limit=1]'
        for saved in ('', ',PortalCooldown:100', ',PortalCooldown:9.75d',
                      ',PortalCooldown:-3', ',PortalCooldown:"bad"'):
            portal.extend([
                cmd(f'/summon {kind} 12.5 234 8.5 {{NoGravity:1b{saved}}}'),
                cmd(f'/data get entity {target} PortalCooldown'),
                {'op': 'tick', 'n': 3},
                cmd(f'/data get entity {target} PortalCooldown'),
                *setup(f'/kill {target}'),
            ])
    portal_reload = [*setup(f'/forceload add {chunk}'), wait]
    for index, kind in enumerate(portal_kinds):
        portal_reload.append(cmd(
            f'/summon {kind} {4100 + 4 * index}.5 300 4104.5 {{NoGravity:1b,PortalCooldown:180}}'))
    portal_reload.append({'op': 'tick', 'n': 3})
    for kind in portal_kinds:
        portal_reload.append(cmd(f'/data get entity @e[type={kind},limit=1] PortalCooldown'))
    portal_reload.extend([
        *setup(f'/forceload remove {chunk}'),
        {'op': 'tick', 'n': 60},
        cmd('/execute unless loaded 4096 300 4096', expect_result=1),
        *setup(f'/forceload add {chunk}'),
        wait,
    ])
    for kind in portal_kinds:
        target = f'@e[type={kind},limit=1]'
        portal_reload.extend([
            cmd(f'/execute if entity {target}', expect_result=1),
            cmd(f'/data get entity {target} PortalCooldown'),
            *setup(f'/kill {target}'),
        ])
    portal_reload.extend(setup(f'/forceload remove {chunk}'))
    return [
        ('projectile common state', summon),
        ('projectile common state reload', reloads),
        ('wither skull chunk reload', skull_reload),
        ('projectile fire and water', block_effects),
        ('other projectile codecs', controls),
        ('Rotation on falling blocks, shulkers and clouds', extra),
        ('Rotation on falling blocks, shulkers and clouds reload', extra_reload),
        ('cloud fang and eye fire', fire),
        ('cloud fang and eye fire reload', fire_reload),
        ('arrow and orb frozen ticks', frozen),
        ('arrow and orb frozen ticks reload', frozen_reload),
        ('arrow and orb portal cooldown', portal),
        ('arrow and orb portal cooldown reload', portal_reload),
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--reference-only', action='store_true')
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    work = (args.work_dir or root / '.local' / 'projectile-common-state-parity').resolve()
    work.mkdir(parents=True, exist_ok=True)
    cases = work / 'groups.json'
    cases.write_text(json.dumps(groups()), encoding='utf-8')
    run = [sys.executable, str(root / 'tools/test_command_exec_parity.py'),
           '--groups-file', str(cases), '--work-dir', str(work)]
    if args.binary:
        run.extend(['--binary', str(args.binary.resolve())])
    if args.prepare:
        run.append('--prepare')
    if args.reference_only:
        run.append('--reference-only')
    return subprocess.run(run, cwd=root).returncode


if __name__ == '__main__':
    sys.exit(main())
