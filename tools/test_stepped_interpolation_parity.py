#!/usr/bin/env python3
"""Compare how a client walks a living entity along its server's positions with vanilla.

Vanilla's unmodified SteppedInterpolationHandler.InterpolationData (VanillaSteppedInterpolationOracle
drives it as the handler does) and horizon-world's SteppedInterpolation replay the same packets and
client ticks: move packets with and without a position or rotation, straight or stepped paths,
repeats of the one the entity is headed for, packets a tick apart that pile up and others after it
stopped, for update intervals of 1 to 3 ticks, plus a punched pig's knockback as the server sends it
(its first move at once, `needsSync`, then every update interval). The entity's position and rotation
after every event must match bit for bit. The handler's entity hooks (deltas, passengers) are out of
scope.
"""
import os
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# A pig punched on flat ground, its positions on the server tick by tick (knockback 0.4 along x).
KNOCKBACK = [(0.0, 0.0), (0.3032, 0.3608), (0.4638, 0.636), (0.6049, 0.8273), (0.7279, 0.9363), (0.8345, 0.9648),
             (0.926, 0.9143), (1.0037, 0.7864), (1.0688, 0.5827), (1.1223, 0.3046), (1.1654, 0.0), (1.1147, 0.0),
             (0.9968, 0.0), (0.8418, 0.0), (0.6664, 0.0)]


def position(rng, p):
    return [round(v + rng.uniform(-0.6, 0.6), rng.choice([2, 4, 17])) for v in p]


def rotation(rng):
    return [round(rng.uniform(-400, 400), rng.choice([0, 1, 3])), round(rng.uniform(-120, 120), rng.choice([0, 2]))]


def cases(count, seed):
    rng = random.Random(seed)
    lines = []
    for interval in (1, 2, 3):
        # The knockback: a packet for the hit tick's move, then every interval (each phase).
        for phase in range(interval):
            lines.append(f'case {interval} 10.5 64.0 0.5 90.0 0.0')
            for k in range(1, len(KNOCKBACK)):
                if k == 1 or (k + phase) % interval == 0:
                    x, y = KNOCKBACK[k]
                    lines.append(f'move L {10.5 + x!r} {64 + y!r} 0.5 -')
                lines.append('tick')
            lines += ['tick'] * 6
    for _ in range(count):
        p = [rng.uniform(-100, 100), rng.uniform(-64, 300), rng.uniform(-100, 100)]
        yaw, pitch = rotation(rng)
        lines.append(f'case {rng.choice([1, 2, 3, 3, 3])} {p[0]!r} {p[1]!r} {p[2]!r} {yaw!r} {pitch!r}')
        last = None
        for _ in range(rng.randrange(1, 40)):
            if rng.random() < 0.45:
                lines.append('tick')
                continue
            if last is not None and rng.random() < 0.15:
                line = last  # the same packet again
            else:
                if rng.random() < 0.1:
                    line = 'move N'
                elif rng.random() < 0.2:
                    n = rng.randrange(1, 5)
                    steps = []
                    for _ in range(n):
                        p = position(rng, p)
                        steps.append(f'{p[0]!r} {p[1]!r} {p[2]!r} {rng.randrange(1, 4)}')
                    line = f'move S {n} ' + ' '.join(steps)
                else:
                    p = position(rng, p) if rng.random() < 0.9 else p
                    line = f'move L {p[0]!r} {p[1]!r} {p[2]!r}'
                rot = rotation(rng) if rng.random() < 0.6 else None
                line += f' R {rot[0]!r} {rot[1]!r}' if rot else ' -'
            last = line
            lines.append(line)
            lines += ['tick'] * rng.choice([0, 1, 1, 2, 3, 6])
    return lines


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'stepped-interpolation-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    events = build / 'events.txt'
    events.write_text('\n'.join(cases(3000, 1374)) + '\n', encoding='utf-8')
    classpath = os.pathsep.join(map(str, [jars['client'], *libraries]))
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSteppedInterpolationOracle.java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-cp', classpath, '-d', str(classes), str(oracle)], check=True)
    subprocess.run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + classpath,
                    'dev.horizonlab.parity.VanillaSteppedInterpolationOracle', str(events), str(build / 'vanilla.txt')],
                   check=True)
    subprocess.run(['cargo', 'run', '--quiet', '--release', '-p', 'horizon-world', '--example',
                    'world_probe', '--', 'stepped_interpolation_fixture', str(events), str(build / 'rust.txt')], cwd=lab.ROOT, check=True)
    matched = base.compare(build / 'vanilla.txt', build / 'rust.txt')
    print(f'Stepped interpolation parity passed: {matched} entity positions and rotations match vanilla')


if __name__ == '__main__':
    main()
