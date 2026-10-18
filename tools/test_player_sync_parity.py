#!/usr/bin/env python3
"""Compare movement packet selection and teleport velocity with the pinned client."""
import json
import random
import struct

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def corpus():
    # Stationary heartbeats, rotation-only updates, status changes and corrected
    # positions followed by unchanged looks must retain vanilla's packet types.
    yield dict(kind="movement", ops=[[0, 0, 0, 0, 0, False, False]] * 61)
    yield dict(kind="movement", ops=[
        [0, 64, 0, 90, 0, True, False],
        [0, 64, 0, 90, 0, True, False],
        [0, 64, 0, 91, 0, True, False],
        [100, 80, -20, 180, 20, False, False],
        [100, 80, -20, 180, 20, True, False],
        [100, 80, -20, 180, 20, True, True],
    ])
    rng = random.Random(0x53594E43)
    for _ in range(300):
        state = [0, 0, 0, 0, 0, False, False]
        ops = []
        for _ in range(80):
            if rng.random() < 0.5:
                axis = rng.randrange(3)
                state[axis] += rng.choice([0, 0.0001, 0.0002, 0.0002000001, 0.1, 10])
            if rng.random() < 0.3:
                state[3] += rng.choice([-360, -1, 0, 0.001, 1, 360])
                state[4] = rng.choice([-90, 0, 30, 90])
            if rng.random() < 0.2:
                state[rng.randrange(5, 7)] = rng.choice([False, True])
            ops.append(state.copy())
        yield dict(kind="movement", ops=ops)
    for flags in range(16):
        for _ in range(40):
            yield dict(kind="velocity", flags=flags << 5,
                       velocity=[rng.uniform(-4, 4) for _ in range(3)],
                       delta=[rng.uniform(-4, 4) for _ in range(3)],
                       old=[rng.uniform(-720, 720), rng.uniform(-90, 90)],
                       new=[rng.uniform(-720, 720), rng.uniform(-90, 90)])


def main():
    def inputs(build):
        rows = list(corpus())
        def bits(v):
            return struct.pack(">d", v).hex()
        for row in rows:
            if row["kind"] == "movement":
                row["ops"] = [[*map(bits, op[:5]), *op[5:]] for op in row["ops"]]
            else:
                for key in ["velocity", "delta", "old", "new"]:
                    row[key] = list(map(bits, row[key]))
        path = build / "inputs.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in rows))
        return path, len(rows)

    harness.differential(
        "player-sync-parity", "VanillaPlayerSyncOracle", "player_sync_fixture", inputs,
        "Actual LocalPlayer.sendPosition packet payloads over persistent movement "
        "sequences; PositionMoveRotation.calculateAbsolute velocity with every "
        "relative-delta/rotate-delta combination. World simulation, connection "
        "scheduling and live anticheat behaviour are outside this harness.",
        sources=[lab.ROOT / "crates/horizon-client/src/player_sync.rs",
                 lab.ROOT / "crates/horizon-client/src/game/remote.rs",
                 lab.ROOT / "crates/horizon-client/src/net.rs"])


if __name__ == "__main__":
    main()
