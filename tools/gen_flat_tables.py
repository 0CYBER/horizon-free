#!/usr/bin/env python3
"""Copy the lookup tables of vanilla's flattening-era DataFixers (1.12.2 -> 1.13.2) into
crates/horizon-world/src/datafix/flat_tables.rs, so that the Rust fixes use exactly vanilla's
data.  Run from the repository root; the vanilla sources are the ones under .local/minecraft.

  gen_flat_tables.py [-o OUT]
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parent.parent
FIXES = lab.GAME / 'sources' / 'server' / 'net' / 'minecraft' / 'util' / 'datafix' / 'fixes'
OUT = ROOT / 'crates' / 'horizon-world' / 'src' / 'datafix' / 'flat_tables.rs'

STR = r'"((?:[^"\\]|\\.)*)"'


def declaration(text, var):
    """The source of `NAME = ...;` for a field called `var`."""
    m = re.search(r'\b' + re.escape(var) + r'\s*=', text)
    if not m:
        sys.exit(f'declaration {var} not found')
    depth = 0
    i = m.end()
    while True:
        c = text[i]
        if c in '({[':
            depth += 1
        elif c in ')}]':
            depth -= 1
        elif c == '"':
            i = text.index('"', i + 1)
            while text[i - 1] == '\\':
                i = text.index('"', i + 1)
        elif c == ';' and depth == 0:
            return text[m.end():i]
        i += 1


def pairs(path, var, value_int=False, key_int=False):
    body = declaration((FIXES / path).read_text(), var)
    key = r'(\d+)' if key_int else STR
    value = r'(\d+)' if value_int else STR
    out = re.findall(r'put\(\s*' + key + r',\s*' + value + r'\s*\)', body)
    if not out:
        sys.exit(f'{path} {var}: no entries')
    return out


def pairs_array(path, var, nullable=False):
    """`map[N] = "x";` entries of an array declaration."""
    text = (FIXES / path).read_text()
    start = text.index(var)
    body = text[start:text.index('});', start)]
    out = re.findall(r'map\[(\d+)\]\s*=\s*' + STR + r';', body)
    if nullable:
        out = [(i, v) for i, v in out]
    if not out:
        sys.exit(f'{path} {var}: no entries')
    return out


def strings(path, var, call):
    body = declaration((FIXES / path).read_text(), var)
    out = re.findall(call + r'\(\s*' + STR + r'\s*\)', body) if call else re.findall(STR, body)
    if not out:
        sys.exit(f'{path} {var}: no entries')
    return out


def rs_str(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def emit_pairs(name, entries, int_key=False, int_value=False, doc=''):
    lines = [f'/// {doc}'] if doc else []
    kt = 'i32' if int_key else '&str'
    vt = 'i32' if int_value else '&str'
    lines.append(f'pub static {name}: &[({kt}, {vt})] = &[')
    for k, v in entries:
        a = k if int_key else rs_str(k)
        b = v if int_value else rs_str(v)
        lines.append(f'    ({a}, {b}),')
    lines.append('];')
    return '\n'.join(lines)


def emit_strs(name, entries, doc=''):
    lines = [f'/// {doc}'] if doc else []
    lines.append(f'pub static {name}: &[&str] = &[')
    lines += [f'    {rs_str(s)},' for s in entries]
    lines.append('];')
    return '\n'.join(lines)


def schema_entities(schema_file, method='registerEntities'):
    """The entity (or block entity) ids a schema's `registerEntities` registers."""
    text = (FIXES.parent / 'schemas' / schema_file).read_text()
    start = text.index(method)
    body = text[start:text.index('return map;', start)]
    names = re.findall(r'(?:registerMob|registerSimple|registerThrowableProjectile|registerMinecart|registerInventory)\(schema, map, "([^"]+)"', body)
    names += re.findall(r'schema\.registerSimple\(map, "([^"]+)"', body)
    names += re.findall(r'schema\.register\(\s*map,\s*"([^"]+)"', body)
    return sorted(set(names))


def structure_renames():
    text = (FIXES / 'ChunkStructuresTemplateRenameFix.java').read_text()
    out = []
    for m in re.finditer(r'\.put\(\s*' + STR + r',\s*Pair\.of\(\s*' + STR + r',\s*ImmutableMap\.<String, String>builder\(\)((?:\s*\.put\(' + STR + r',\s*' + STR + r'\))*)\s*\.build\(\)', text):
        structure, child = m.group(1), m.group(2)
        entries = re.findall(r'\.put\(' + STR + r',\s*' + STR + r'\)', m.group(3))
        out.append((structure, child, entries))
    if len(out) != 4:
        sys.exit(f'expected 4 structure renames, found {len(out)}')
    lines = ['/// `ChunkStructuresTemplateRenameFix.RENAMES`: structure id, child id, template renames.',
             'pub static STRUCTURE_TEMPLATE_RENAMES: &[(&str, &str, &[(&str, &str)])] = &[']
    for structure, child, entries in out:
        lines.append(f'    ({rs_str(structure)}, {rs_str(child)}, &[')
        lines += [f'        ({rs_str(a)}, {rs_str(b)}),' for a, b in entries]
        lines.append('    ]),')
    lines.append('];')
    return '\n'.join(lines)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('-o', '--out', default=str(OUT))
    args = p.parse_args()
    parts = [
        '//! Lookup tables of the DataFixers of 1.13 (DataVersion 1343 to 1631), copied from vanilla\'s\n'
        '//! sources by `tools/gen_flat_tables.py`. Do not edit.\n',
        emit_pairs('ITEM_FLATTENING', pairs('ItemStackTheFlatteningFix.java', 'MAP'),
                   doc='`ItemStackTheFlatteningFix.MAP`: `id.damage` to the flattened item.'),
        emit_strs('ITEM_DAMAGE_IDS', strings('ItemStackTheFlatteningFix.java', 'DAMAGE_IDS', ''),
                  doc='`ItemStackTheFlatteningFix.DAMAGE_IDS`: items that keep their damage in `tag.Damage`.'),
        emit_pairs('BLOCK_IDS', pairs('EntityBlockStateFix.java', 'MAP', value_int=True), int_value=True,
                   doc='`EntityBlockStateFix.MAP`: 1.12 block name to numeric id.'),
        emit_pairs('ITEM_IDS', pairs('ItemIdFix.java', 'ITEM_NAMES', key_int=True), int_key=True,
                   doc='`ItemIdFix.ITEM_NAMES`: numeric item id to name.'),
        emit_strs('SPECIAL_OBJECTIVE_CRITERIA', strings('StatsCounterFix.java', 'SPECIAL_OBJECTIVE_CRITERIA', '')),
        emit_strs('STATS_SKIP', strings('StatsCounterFix.java', 'SKIP', 'add')),
        emit_pairs('STATS_CUSTOM', pairs('StatsCounterFix.java', 'CUSTOM_MAP')),
        emit_pairs('STATS_ITEM_KEYS', pairs('StatsCounterFix.java', 'ITEM_KEYS')),
        emit_pairs('STATS_ENTITY_KEYS', pairs('StatsCounterFix.java', 'ENTITY_KEYS')),
        emit_pairs('STATS_ENTITIES', pairs('StatsCounterFix.java', 'ENTITIES')),
        emit_pairs('RECIPES', pairs('RecipesFix.java', 'RECIPES')),
        emit_pairs('RECIPES_RENAMENING', pairs('RecipesRenameningFix.java', 'RECIPES')),
        emit_pairs('ADVANCEMENTS', pairs('AdvancementsFix.java', 'RENAMES')),
        emit_pairs('GENERATOR_BIOMES', pairs('LevelDataGeneratorOptionsFix.java', 'MAP')),
        emit_pairs('SPAWN_EGGS', pairs('ItemStackSpawnEggFix.java', 'MAP')),
        emit_pairs('ENCHANTMENTS', pairs('ItemStackEnchantmentNamesFix.java', 'MAP', key_int=True), int_key=True),
        emit_pairs('CORAL', pairs('RenamedCoralFix.java', 'RENAMED_IDS')),
        emit_pairs('CORAL_FANS', pairs('RenamedCoralFansFix.java', 'RENAMED_IDS')),
        emit_pairs('THE_RENAMENING_ENTITIES', pairs('EntityTheRenameningFix.java', 'RENAMED_IDS')),
        emit_pairs('THE_RENAMENING_BLOCKS', pairs('EntityTheRenameningFix.java', 'RENAMED_BLOCKS')),
        emit_pairs('COD_SALMON', pairs('EntityCodSalmonFix.java', 'RENAMED_IDS')),
        emit_pairs('COD_SALMON_EGGS', pairs('EntityCodSalmonFix.java', 'RENAMED_EGG_IDS')),
        emit_strs('ENTITIES_V1460', schema_entities('V1460.java'),
                  doc='The entity ids V1460 registers (V705 has the same ones).'),
        emit_strs('ENTITIES_V99', schema_entities('V99.java'), doc='The entity ids V99 registers.'),
        emit_strs('BLOCK_ENTITIES_V99', schema_entities('V99.java', 'registerBlockEntities'),
                  doc='The block entity ids V99 registers.'),
        emit_pairs('SPAWN_EGG_ENTITIES', pairs_array('ItemSpawnEggFix.java', 'ID_TO_ENTITY'), int_key=True,
                   doc='`ItemSpawnEggFix.ID_TO_ENTITY`: spawn egg damage to entity id.'),
        emit_pairs('ENTITY_IDS', pairs('EntityIdFix.java', 'ID_MAP'), doc='`EntityIdFix.ID_MAP`.'),
        emit_pairs('BLOCK_ENTITY_IDS', pairs('BlockEntityIdFix.java', 'ID_MAP'), doc='`BlockEntityIdFix.ID_MAP`.'),
        emit_pairs('ITEM_TO_BLOCK_ENTITY_V99', pairs('../schemas/V99.java', 'ITEM_TO_BLOCKENTITY'),
                   doc='`V99.ITEM_TO_BLOCKENTITY`.'),
        emit_pairs('ITEM_POTIONS', pairs_array('ItemPotionFix.java', 'POTIONS', nullable=True), int_key=True,
                   doc='`ItemPotionFix.POTIONS`: damage & 127 to potion (absent: null).'),
        structure_renames(),
    ]
    Path(args.out).write_text('\n\n'.join(parts) + '\n')
    subprocess.run(['rustfmt', '--edition', '2024', args.out], check=True)
    print('wrote', args.out)


if __name__ == '__main__':
    main()
