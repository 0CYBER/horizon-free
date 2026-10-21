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
        # Minecraft.crosshairPickEntity (a living entity, alive or dead) and
        # getCurrentItemAttackStrengthDelay, for the full indicator.
        'target': rng.choice([None, True, True, False]),
        'attackDelay': rng.choice([5.0, 12.5, 4.0, 20.0, 5.0001, 10.0]),
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
    if case['target'] and rng.random() < 0.5:
        case['attackScale'] = 1.0
    if case['damageCooldown'] and rng.random() < 0.5:
        case['healthBlinkTime'] = case['tick'] + rng.randint(1, 20)
    if rng.random() < 0.15:
        case['vehicleMaxHealth'] = rng.choice([15.0, 30.0, 53.0, 100.0])
        case['vehicleHealth'] = round(rng.uniform(0, case['vehicleMaxHealth']), 1)
    return case


ALL_EFFECTS = ['speed', 'slowness', 'haste', 'mining_fatigue', 'strength', 'instant_health', 'instant_damage', 'jump_boost',
               'nausea', 'regeneration', 'resistance', 'fire_resistance', 'water_breathing', 'invisibility', 'blindness',
               'night_vision', 'hunger', 'weakness', 'poison', 'wither', 'health_boost', 'absorption', 'saturation', 'glowing',
               'levitation', 'luck', 'unluck', 'slow_falling', 'conduit_power', 'dolphins_grace', 'bad_omen',
               'hero_of_the_village', 'darkness', 'trial_omen', 'raid_omen', 'wind_charged', 'weaving', 'oozing', 'infested',
               'breath_of_the_nautilus']
SIZES = dict(width=[427, 320, 480, 640, 961], height=[240, 255, 270, 360, 501])


def screen(rng):
    return {'width': rng.choice(SIZES['width']), 'height': rng.choice(SIZES['height'])}


def backdrop(rng):
    """The text background options behind textWithBackdrop."""
    return {'chatOnly': rng.random() < 0.5, 'textOpacity': rng.choice([0.0, 0.5, 0.25, 1.0, round(rng.random(), 3)])}


def effects(rng, n):
    count = rng.choice([1, 2, 3, 5, 8, 12, 20])
    active = []
    for e in rng.sample(ALL_EFFECTS, count):
        duration = rng.choice([-1, 1, 7, 19, 20, 99, 150, 199, 200, 201, 600, 32147, 32148, 40000, rng.randint(0, 220)])
        active.append({'id': 'minecraft:' + e, 'duration': duration, 'ambient': rng.random() < 0.25, 'icon': rng.random() < 0.9})
    return {**screen(rng), 'demo': rng.random() < 0.15, 'active': active}


TITLES = [{'text': 'Hello'}, {'text': 'Boss fight!', 'color': 'red', 'bold': True}, {'text': 'Wave 3', 'italic': True},
          {'text': 'A long title over the whole screen'}, {'text': ''}, ['Mixed ', {'text': 'parts', 'color': 'gold'}],
          {'text': 'Ünïcödé ★'},
          # Servers send legacy formatting codes inside a component's text; the font applies them:
          {'text': '§8[§fAny questions? §8] §e§l/support en'}, {'text': '§cRed §rreset §', 'color': 'blue'}]


def title(rng, n):
    """Title, subtitle and action bar events between ticks (fades, setTimes mid-title, clears)."""
    ops = []
    for _ in range(rng.randint(1, 6)):
        r = rng.random()
        if r < 0.3:
            ops.append(['title', rng.choice(TITLES)])
        elif r < 0.45:
            ops.append(['subtitle', rng.choice(TITLES)])
        elif r < 0.6:
            ops.append(['times', rng.choice([-1, 0, 1, 5, 10, 20]), rng.choice([-1, 0, 3, 40, 70]), rng.choice([-1, 0, 1, 7, 20])])
        elif r < 0.65:
            ops.append(['clear'])
        elif r < 0.7:
            ops.append(['reset'])
        elif r < 0.85:
            ops.append(['overlay', rng.choice(TITLES), rng.random() < 0.3])
        ops.append(['tick', rng.choice([0, 0, 1, 2, 5, 9, 10, 11, 30, 50, 59, 60, 79, 80, 81, 99, 100, rng.randint(0, 120)])])
    return {**screen(rng), **backdrop(rng), 'partial': round(rng.random(), 3), 'ops': ops}


# Items for the selected item name: id, translation key and rarity colour (checked by the oracle's own stacks).
HELD = [('stone', 'block.minecraft.stone', 'white'), ('diamond_sword', 'item.minecraft.diamond_sword', 'white'),
        ('totem_of_undying', 'item.minecraft.totem_of_undying', 'yellow'), ('beacon', 'block.minecraft.beacon', 'aqua'),
        ('dragon_egg', 'block.minecraft.dragon_egg', 'light_purple'), ('apple', 'item.minecraft.apple', 'white')]
CUSTOM = [None, None, None, {'text': 'Excalibur'}, {'text': 'Red', 'color': 'red'}, {'text': 'Plain', 'italic': False}]


def selected(rng, n):
    """Selected-slot changes tick by tick, then the name with its fade."""
    def stack():
        item, key, rarity = rng.choice(HELD)
        return {'item': item, 'key': key, 'rarity': rarity, 'custom': rng.choice(CUSTOM)}
    steps, held = [], stack()
    for _ in range(rng.randint(1, 60)):
        if rng.random() < 0.1:
            if rng.random() < 0.2:
                held = None
            else:
                held = stack()
        steps.append(held)
    return {**screen(rng), **backdrop(rng), 'gameMode': rng.choice(['survival', 'adventure', 'creative']),
            'displayTime': rng.choice([1.0, 1.0, 0.5, 2.0, 0.123]), 'steps': steps}



def boss(rng, n):
    bars = []
    for _ in range(rng.choice([1, 1, 2, 3, 5, 9])):
        progress = rng.choice([0.0, 1.0, 0.5, 0.001, 0.999, 1 / 182, round(rng.random(), 4)])
        bars.append({'name': rng.choice(TITLES), 'progress': progress, 'color': rng.randrange(7), 'overlay': rng.randrange(5)})
    return {**screen(rng), 'bars': bars}


def jump(rng, n):
    return {**screen(rng), 'cooldown': rng.choice([0, 0, 0, 1, 10]),
            'jumpScale': rng.choice([0.0, 1.0, 0.4, 0.001, round(rng.random(), 4)])}


# Servers' sidebars use legacy formatting codes in fake owners, team affixes and titles (#937):
# a code, an unknown code, `§r` and a trailing lone `§`.
OWNERS = ['Alex', 'steve', 'Zed', 'bob', 'Bobby', 'ÉLodie', 'élan', '#hidden', 'x', 'LongPlayerName_123', 'Mia', '_under',
          '§1§r', '§a§lPro§r name', 'end§', '§zodd§Rcase']
FORMATS = [None, None, None, {'type': 'blank'}, {'type': 'fixed', 'value': {'text': 'FIX', 'color': 'aqua'}},
           {'type': 'styled', 'style': {'color': 'gold', 'bold': True}}, {'type': 'styled', 'style': {}}]


def sidebar(rng, n):
    """An objective with scores, display names, number formats and teams."""
    owners = rng.sample(OWNERS, rng.randint(0, len(OWNERS)))
    if rng.random() < 0.3:  # more rows than the 15 shown
        owners += [f'p{i}' for i in range(rng.randint(5, 20))]
    scores = [{'owner': o, 'value': rng.choice([0, 1, -1, 7, 7, 100, 2147483647, -2147483648, rng.randint(-50, 50)]),
               'display': rng.choice([None, None, None, {'text': 'Shown ' + o, 'color': 'green'}]),
               'format': rng.choice(FORMATS)} for o in owners]
    teams = []
    for t in range(rng.choice([0, 0, 1, 2])):
        teams.append({'name': f'team{t}', 'prefix': rng.choice([{'text': ''}, {'text': '[A] '}, {'text': '<', 'color': 'red'},
                                                                {'text': '§c§lRed §r', 'color': 'blue'}]),
                      'suffix': rng.choice([{'text': ''}, {'text': ' *'}, {'text': ' §7(§e5§7)§'}]),
                      'color': rng.choice([None, 'red', 'blue', 'gold', 'white']),
                      'members': [o for o in owners if rng.random() < 0.3]})
    return {**screen(rng), **backdrop(rng),
            'objective': {'display': rng.choice(TITLES + [{'text': 'Kills'}, {'text': '§e§lANTICHEAT §7test'}]),
                          'format': rng.choice(FORMATS + [{'type': 'fixed', 'value': {'text': '§a12§'}}])},
            'scores': scores, 'teams': teams}



# Items for the decorations: id and max damage (0 = not damageable), checked by the oracle's own stacks.
DECOR_ITEMS = [('diamond_sword', 1561), ('bow', 384), ('shield', 336), ('flint_and_steel', 64), ('elytra', 432),
               ('stone', 0), ('ender_pearl', 0), ('apple', 0)]


def decor(rng, n):
    """GuiGraphicsExtractor.itemDecorations: count, durability bar and cooldown shade."""
    item, max_damage = rng.choice(DECOR_ITEMS)
    case = {'x': rng.randint(-20, 400), 'y': rng.randint(-20, 300), 'item': item, 'maxDamage': max_damage,
            'count': rng.choice([1, 1, 2, 16, 64, 99, 0, 10, 127]),
            'damage': rng.choice([0, 0, 1, max_damage // 2, max_damage - 1, max_damage, rng.randint(0, max(max_damage, 1))]) if max_damage else 0,
            'unbreakable': rng.random() < 0.1, 'partial': round(rng.random(), 3)}
    if rng.random() < 0.4:
        duration = rng.choice([1, 2, 7, 20, 100, 200])
        case['cooldown'] = {'duration': duration, 'elapsed': rng.choice([0, 1, duration - 1, duration, rng.randint(0, duration)])}
    if rng.random() < 0.1:
        case['countText'] = rng.choice(['', '+', '1/2', 'x9'])
    return case



def overlays(rng, n):
    """Camera overlays (vignette and border warning, spyglass, pumpkin, powder snow, portal, nausea) and sleep."""
    case = {**screen(rng), 'vignette': rng.random() < 0.7,
            'vignetteBrightness': rng.choice([1.0, 0.0, 0.5, round(rng.random(), 4), 1.3, -0.2]),
            'firstPerson': rng.random() < 0.8, 'scoping': rng.random() < 0.2,
            'scopeScale': rng.choice([0.5, 1.125, round(rng.uniform(0.5, 1.125), 4)]), 'partial': round(rng.random(), 3),
            'head': rng.choice([None, None, 'carved_pumpkin', 'diamond_helmet']),
            'ticksFrozen': rng.choice([0, 0, 0, 1, 70, 139, 140, 300]),
            'portal': rng.choice([[0.0, 0.0], [0.0, 0.0], [0.0, 0.0125], [0.5, 0.5125], [1.0, 1.0], [0.05, 0.0], [round(rng.random(), 3)] * 2]),
            'nausea': rng.choice([0.0, 0.0, 1.0, 0.25, round(rng.random(), 3)]),
            'screenEffectScale': rng.choice([1.0, 0.0, 0.5, round(rng.random(), 3)]),
            'sleepTimer': rng.choice([0, 0, 0, 1, 50, 100, 101, 105, 110, 111])}
    if rng.random() < 0.4:
        size = rng.choice([10.0, 50.0, 200.0, 1000.0])
        case['border'] = {'centerX': rng.choice([0.0, 100.5]), 'centerZ': rng.choice([0.0, -30.25]), 'size': size,
                          'warningBlocks': rng.choice([0, 5, 20, 100]), 'warningTime': rng.choice([0, 15])}
    case['camX'] = round(rng.uniform(-150, 250), 2)
    case['camZ'] = round(rng.uniform(-150, 150), 2)
    return case



WORDS = ['hello', 'world', 'a', 'Minecraft', 'supercalifragilisticexpialidocious', 'x' * 70, 'ok!', 'Ünïcödé', '★', '']


def chat_text(rng):
    words = [rng.choice(WORDS) for _ in range(rng.choice([1, 2, 5, 12, 30]))]
    text = rng.choice([' ', ' ', ' ', '  ', '\n']).join(words)
    if rng.random() < 0.1:
        text = rng.choice([' ', '\n']) + text + rng.choice(['', ' ', '\n'])
    if rng.random() < 0.3:
        cut = rng.randint(0, len(text))
        return [{'text': text[:cut], 'color': rng.choice(['red', 'gold', 'aqua'])},
                {'text': text[cut:], 'bold': rng.random() < 0.5, 'italic': rng.random() < 0.3}]
    return {'text': text}


def chat(rng, n):
    """Messages wrapped to the chat width and fading 200 ticks after they arrive."""
    count = rng.choice([1, 2, 5, 10, 30, 120])
    now = rng.randint(200, 2000)
    messages = []
    for _ in range(count):
        at = rng.choice([now, now - 1, now - 179, now - 180, now - 190, now - 199, now - 200, now - 500, rng.randint(now - 260, now + 5)])
        messages.append({'text': chat_text(rng), 'at': at, 'tag': rng.choice([None, None, 'system', 'error'])})
    messages.sort(key=lambda m: m['at'])
    return {**screen(rng), 'now': now, 'messages': messages,
            'chatScale': rng.choice([1.0, 1.0, 0.5, 0.0 + 0.75, 0.25]), 'chatWidth': rng.choice([1.0, 1.0, 0.5, 0.0, 0.3]),
            'chatOpacity': rng.choice([1.0, 0.5, 0.0, 0.25]), 'textOpacity': rng.choice([0.5, 0.0, 1.0, 0.25]),
            'chatLineSpacing': rng.choice([0.0, 0.0, 0.5, 1.0, 0.25]), 'chatHeightUnfocused': rng.choice([70 / 160, 1.0, 0.0, 0.5]),
            'chatHeightFocused': rng.choice([1.0, 1.0, 0.5, 0.25]),
            # The chat screen open: foreground, focused height, scrolled up or down.
            'focused': rng.random() < 0.5, 'scroll': rng.choice([0, 0, 1, 3, 7, 20, -2, 1000])}


# Generators by kind: (rng, index) -> case. Cases are drawn round-robin.
KINDS = {
    'decorations': decorations,
    'effects': effects,
    'title': title,
    'selected': selected,
    'boss': boss,
    'jump': jump,
    'sidebar': sidebar,
    'decor': decor,
    'overlays': overlays,
    'chat': chat,
}
