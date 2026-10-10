"""Random HUD cases for test_hud_parity.py, one generator per case kind."""

EFFECTS = ['minecraft:poison', 'minecraft:wither', 'minecraft:regeneration', 'minecraft:hunger', 'minecraft:speed']


def decorations(rng, n):
    max_health = rng.choice([20.0, 20.0, 6.0, 40.0, 33.0, 100.0])
    health = rng.choice([max_health, round(rng.uniform(0, max_health), 2), 1.0, 0.5, 3.0])
    air_max = 300
    under = rng.random() < 0.3
    air = rng.choice([air_max, rng.randint(-20, air_max), 0, 1, 29, 30, 31, 299]) if under or rng.random() < 0.2 else air_max
    case = {
        'width': rng.choice([427, 320, 480, 640, 961]), 'height': rng.choice([240, 255, 270, 360, 501]),
        'gameMode': rng.choice(['survival'] * 5 + ['adventure', 'creative', 'spectator']),
        'crosshair': rng.random() < 0.7,
        'attackIndicator': rng.choice(['OFF', 'CROSSHAIR', 'HOTBAR']),
        'attackScale': rng.choice([1.0, round(rng.random(), 3), 0.0, 0.999]),
        'selected': rng.randint(0, 8), 'rightHanded': rng.random() < 0.7,
        'slots': [rng.choice([None, 0, 0, 3, 5]) if rng.random() < 0.6 else None for _ in range(9)],
        'offhand': rng.choice([None, None, 0, 4]), 'partial': round(rng.random(), 3),
        'health': health, 'maxHealth': max_health,
        'absorption': rng.choice([0.0, 0.0, 4.0, 3.5, 17.0, 40.0]),
        'armor': rng.choice([0, 0, 1, 7, 20, 13]), 'food': rng.randint(0, 20),
        'saturation': rng.choice([0.0, 5.0, 0.0]),
        'effects': [e for e in EFFECTS if rng.random() < 0.15], 'frozen': rng.random() < 0.08,
        'hardcore': rng.random() < 0.2, 'damageCooldown': rng.choice([0, 0, 10]),
        'air': air, 'maxAir': air_max, 'underWater': under,
        'tick': rng.randint(0, 100000), 'lastHealth': rng.randint(0, 40), 'displayHealth': rng.randint(0, 40),
        'lastHealthAgo': rng.choice([0, 500, 5000]), 'healthBlinkTime': 0,
        'xpProgress': rng.choice([0.0, round(rng.random(), 4), 0.999]), 'xpLevel': rng.choice([0, 1, 7, 30, 123, 4000]),
        'experienceBar': rng.random() < 0.8,
    }
    if case['damageCooldown'] and rng.random() < 0.5:
        case['healthBlinkTime'] = case['tick'] + rng.randint(1, 20)
    if rng.random() < 0.15:
        case['vehicleMaxHealth'] = rng.choice([15.0, 30.0, 53.0, 100.0])
        case['vehicleHealth'] = round(rng.uniform(0, case['vehicleMaxHealth']), 1)


    return case


# Generators by kind: (rng, index) -> case. Cases are drawn round-robin.
KINDS = {
    'decorations': decorations,
}
