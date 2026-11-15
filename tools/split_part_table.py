#!/usr/bin/env python3
"""One-shot arismoko/horizon#1459 move; --verify BASE checks exact items, order and moved lines.

Declared edits: repeated trait/impl/module envelopes, one generated include, the
shared support comment, and rustfmt whitespace. No body or signature substitutions.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path('crates/horizon-mod-vocab/src/part_table.rs')
DIRECTORY = SOURCE.with_suffix('')
INCLUDE = 'include!(concat!(env!("OUT_DIR"), "/part_table_generated.rs"));\n'
COMMENT = '//! Shared table state, cross-kind registration and common test fixtures only.\n'
# Original cohesive fixture sections, anchored by declaration rather than line number.
TEST_AREAS = {
    'cross_mod_categories_resolve_after_all_registration': 'game_rules',
    'register_game_keys': 'shared', 'Gauge': 'blocks',
    'entity_parts_refuse_unnamespaced_ids_and_doubles': 'entities',
    'invalid_and_duplicate_fluid_declarations_are_refused': 'fluids',
    'item_keys_validate_forward_references_and_reused_slots': 'items',
    'registry_keys_resolve_reused_parts_and_rebind_without_changing_identity': 'blocks',
    'registry_keys_resolve_effects_declared_later_and_generated_name_reads': 'effects',
    'block_entity_slots_resolve_forward_declarations_and_refuse_unknown': 'block_entities',
    'registry_keys_reject_unknown_unattached_requests_with_the_requesting_mod': 'shared',
    'invalid_block_entity_type_paths_refuse_registration': 'block_entities',
    'invalid_lookup_ids_refuse_registration_naming_the_requesting_mod': 'shared',
    'duplicate_and_foreign_structure_types_refuse_registration': 'structures',
    'Connector': 'blocks', 'Press': 'block_entities', 'NoHost': 'shared', 'TagGauge': 'blocks',
    'Doubler': 'shared', 'Pipe': 'items', 'Level': 'shared', 'Chemical': 'capabilities',
    'EntityBattery': 'entities', 'Chemist': 'capabilities', 'FixtureContent': 'shared',
    'signal_game_thread': 'blocks', 'a_level_not_in_place_asks_no_provider': 'capabilities',
    'an_item_handler_writes_its_stack_back_where_it_lives_splitting_one_off': 'items',
    'ticket_controllers_get_the_callers_namespace_and_refuse_anothers': 'tickets',
    'key_mappings_are_owned_and_duplicates_refuse_startup': 'key_mappings',
    'a_ticket_controller_registered_twice_refuses_the_mods_naming_mod_and_id': 'tickets',
    'a_place_provider_given_twice_is_refused': 'items',
    'a_capability_name_answered_with_two_handler_types_is_refused': 'capabilities',
    'disabled_place_providers_refuse_reads_and_writes_and_resume': 'items',
    'providers_are_asked_ahead_of_the_game_then_the_game_then_the_mods_in_order': 'capabilities',
    'TypeOf': 'shared', 'HeldItem': 'items', 'Lie': 'shared', 'ModTank': 'fluids',
    'ModBattery': 'energy', 'a_mods_tag_part_registered_first_runs_before_its_key_part': 'blocks',
    'a_vanilla_type_refuses_a_native_payload_part_at_registration': 'block_entities',
    'with_no_parts_the_block_entity_is_never_named': 'capabilities',
    'the_same_part_twice_on_one_block_is_refused': 'blocks',
    
}


def run(*args, source=None):
    return subprocess.check_output(args, cwd=ROOT, input=source, text=True)


def formatted(source):
    return run('rustfmt', '--edition', '2024', '--config', 'skip_children=true',
               '--emit', 'stdout', source=source)


def area(text):
    declaration = re.search(r'^(?:(?:pub(?:\([^)]*\))? )?(?:fn|trait|struct|enum|type) |impl[ <])[^\n]+', text, re.M)
    name = re.search(r'\bfn (\w+)', text) if declaration is None or re.match(r'(?:pub(?:\([^)]*\))? )?fn ', declaration[0]) else None
    name = name[1] if name else declaration[0] if declaration else text
    for pattern, kind in [
        (r'game_rule', 'game_rules'), (r'key_mapping', 'key_mappings'),
        (r'ticket|controller', 'tickets'), (r'structure', 'structures'),
        (r'place_provider|PlaceProviderPart|tooltip|Tooltip', 'items'),
        (r'block_entity|BlockEntity|BlockCapabilityProvider|BlockProvider', 'block_entities'),
        (r'entity|Entity', 'entities'), (r'item|ItemKeyError|ItemCapabilityProvider|ItemProvider', 'items'),
        (r'fluid', 'fluids'), (r'effect', 'effects'), (r'block|BlockProvider', 'blocks'),
        (r'menu', 'menus'),
        (r'capability|Capability|Capabilities|Provider|providers|provided|^next$|^answered$|Answer', 'capabilities')]:
        if re.search(pattern, name):
            return kind
    return 'shared'


def split(source, layout):
    lines = source.splitlines(keepends=True)
    outputs = defaultdict(str)
    first = next(i['start'] - 1 for i in layout if i['start'] > 25)
    residual = ''.join(lines[:first]) + '\n' + INCLUDE
    cursor = first
    for item in layout:
        if item['end'] <= first:
            continue
        start, end = item['start'] - 1, item['end']
        prefix = ''.join(lines[cursor:start])
        cursor = end
        text = ''.join(lines[start:end])
        if 'children' in item and 'pub mod tests {' in text:
            members = item['children']
            groups = defaultdict(str)
            previous = next(n for n in range(start, end) if lines[n].endswith('pub mod tests {\n')) + 1
            header = ''.join(lines[start:previous])
            kind = 'shared'
            for member in members:
                text = ''.join(lines[previous:member['end']])
                declaration = re.search(r'^    (?:pub )?(?:fn|struct|enum|trait|mod) (\w+)', text, re.M)
                if declaration:
                    kind = TEST_AREAS.get(declaration[1], kind)
                target = 'shared' if not declaration and re.search(r'^    use ', text, re.M) else kind
                if declaration and declaration[1] == 'tooltip_appender_restores_owner_even_on_panic':
                    target = 'items'
                groups[target] += text
                previous = member['end']
            groups['shared'] += ''.join(lines[previous:end - 1])
            for kind, body in groups.items():
                outputs[kind] += prefix + header + body + '}\n'
        elif ('members' in item and re.search(r'^(?:pub trait (?:Registrar|CapabilityProviders)\b|impl (?:dyn Registrar|Registrar for PartRegistrar|PartRegistrar|HookParts|Parts|CapabilityProviders for InstalledProviders)\b)', text, re.M)):
            members = item['members']
            previous = members[0]['start'] - 1
            while not lines[previous - 1].rstrip().endswith('{'):
                previous -= 1
            header = ''.join(lines[start:previous])
            groups = defaultdict(str)
            for member in members:
                kind = area(member['text'])
                groups[kind] += ''.join(lines[previous:member['end']])
                previous = member['end']
            groups['shared'] += ''.join(lines[previous:end - 1])
            for kind, body in groups.items():
                if body.strip():
                    outputs[kind] += prefix + header + body + '}\n'
        else:
            # Cross-kind types and helpers retain the original shared table scope.
            kind = area(text)
            if re.search(r'^(?:pub(?:\([^)]*\))? )?(?:struct|enum|fn|mod) (?:Installed|Parts|HookParts|PartRegistrar|register_all|install_registrar|resolve_parts|unknown_keys|add_tagged|attached)\b', text, re.M):
                kind = 'shared'
            outputs[kind] += prefix + text
    outputs['shared'] = COMMENT + outputs['shared']
    return formatted(residual), {kind + '.rs': formatted(text) for kind, text in outputs.items()}


def payload(source, layout):
    lines = source.splitlines()
    excluded = set()
    def walk(items):
        for item in items:
            members = item.get('members', item.get('children'))
            if members:
                first = members[0]['start'] - 1
                while first > item['start'] and not lines[first - 1].rstrip().endswith('{'):
                    first -= 1
                excluded.update(range(item['start'] - 1, first))
                excluded.add(item['end'] - 1)
                if 'children' in item:
                    walk(item['children'])
    walk(layout)
    return Counter(line.strip() for n, line in enumerate(lines) if n not in excluded
                   and line.strip() and line != COMMENT.rstrip())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify', metavar='BASE')
    parser.add_argument('--base', metavar='BASE')
    args = parser.parse_args()
    run('cargo', 'build', '-q', '-p', 'horizon-mod-api-gen', '--bin', 'part-table-layout')
    metadata = json.loads(run('cargo', 'metadata', '--no-deps', '--format-version', '1'))
    binary = Path(metadata['target_directory']) / 'debug' / 'part-table-layout'
    layout = lambda source: json.loads(run(str(binary), source=source))
    base = args.verify or args.base
    source = run('git', 'show', f'{base}:{SOURCE}') if base else (ROOT / SOURCE).read_text()
    residual, files = split(source, layout(source))
    if args.verify:
        actual = {p.name: p.read_text() for p in (ROOT / DIRECTORY).glob('*.rs')}
        assert actual == files, 'item contents, envelopes or within-kind order differ'
        assert (ROOT / SOURCE).read_text() == residual, 'root drift'
        before = payload(source, layout(source))
        after = payload(residual, layout(residual))
        after.subtract(Counter([INCLUDE.strip()]))
        for text in files.values():
            after.update(payload(text, layout(text)))
        assert +before == +after, f'moved lines differ: removed {before - after}; added {after - before}'
        print(f'Moved payload line multisets equal: {sum(before.values())} nonblank lines')
        print('Exact formatted items, attributes, signatures, bodies and within-kind order verified')
    else:
        (ROOT / DIRECTORY).mkdir(exist_ok=True)
        (ROOT / SOURCE).write_text(residual)
        for name, text in files.items():
            (ROOT / DIRECTORY / name).write_text(text)
    for name, text in sorted(files.items()):
        fixtures = sum(i['end'] - i['start'] + 1 for i in layout(text)
                       if 'children' in i and 'pub mod tests {' in '\n'.join(text.splitlines()[i['start'] - 1:i['end']]))
        print(f'{name}: {len(text.splitlines())} lines; {fixtures} fixture-module lines')


if __name__ == '__main__':
    main()
