#!/usr/bin/env python3
"""First-pass scaffolding of missing mod_api types, written as ordinary hand-editable source.

For each engine type in the allow-list (api_scaffold.toml) it adds, never overwriting:
  * the API type to crates/horizon-mod-vocab/src/<domain>/<topic>.rs (a mirror, or an `XRef` handle);
  * its re-export to src/exports/<topic>.rs;
  * for a handle with methods, a `<Name>Backend` host area to src/host/<topic>.rs;
  * both conversions to crates/horizon-client/src/game/modding/api_values/<topic>.rs.
It refuses when the type or its `<Name>Backend` already exists in crates/horizon-mod-vocab/src, and never
targets lib.rs. Conversions in the client crate are not checked. Every emitted value explicitly derives `Wire`; handles also get a `Fields` impl.

  api_scaffold.py --dry-run [--inventory points-inventory-runtime.json] [--report out.md]
  api_scaffold.py --apply
"""
import argparse
import difflib
import re
import sys
import tomllib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
API = Path('crates/horizon-mod-vocab/src')
CONV = Path('crates/horizon-client/src/game/modding/api_values')
DOMAINS = ('world', 'block', 'item', 'entity', 'client', 'command', 'text', 'content', 'wire')
SKIP = set('Option Vec Result R Box Self dyn mut str String Path static Game Host super std path crate fn impl '
           'Fn FnMut FnOnce Cow HashMap HashSet Rc Arc i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 usize isize '
           'f32 f64 bool char'.split())


class Refuse(Exception):
    pass


KEYWORDS = set('as break const continue crate else enum extern false fn for if impl in let loop match mod move mut '
               'pub ref return self Self static struct super trait true type unsafe use where while async await dyn '
               'abstract become box do final macro override priv typeof unsized virtual yield try gen'.split())
IDENT = re.compile(r'[A-Za-z][A-Za-z0-9_]*$')


def ident(what, value):
    if not isinstance(value, str) or not IDENT.match(value) or value in KEYWORDS:
        raise Refuse(f'{what} {value!r} is not a plain Rust identifier')
    return value


def check_entry(e):
    """Validate an allow-list entry once, where it enters; everything after trusts it."""
    if e.get('kind') not in ('struct', 'enum', 'handle'):
        raise Refuse(f"{e.get('engine')}: kind must be struct, enum or handle")
    if e.get('domain') not in DOMAINS:
        raise Refuse(f"domain {e.get('domain')!r} must be one of {', '.join(sorted(DOMAINS))}")
    ident('topic', e.get('topic'))
    if e['topic'] != e['topic'].lower():
        raise Refuse(f"topic {e['topic']!r} must be a lower-case module name")
    name = ident('api name', api_name(e))
    if name.startswith('Native'):
        raise Refuse(f'api name {name!r} must not use the Native prefix')
    for v in e.get('variants', []):
        ident('variant', v)
    for f in e.get('fields', []):
        ident('field', split_field(f)[0])
    for m in e.get('methods', []):
        ident('method', m['name'])


def snake(name):
    return re.sub(r'(?<!^)(?=[A-Z])', '_', name).lower()


def split_field(spec):
    name, ty = (s.strip() for s in spec.split(':', 1))
    return name, ty


def api_name(entry):
    return entry.get('name') or (entry['engine'].split('::')[-1].removeprefix('Native')
                                 + ('Ref' if entry['kind'] == 'handle' else ''))


def mirror_source(e, name):
    exhaustive = e.get('exhaustive')
    if e['kind'] == 'enum':
        body = ''.join(f'    /// `{v}`.\n    {v},\n' for v in e['variants'])
        return (f"/// {e['doc']}\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]\n#[non_exhaustive]\n"
                f'pub enum {name} {{\n{body}}}\n')
    fields = [split_field(f) for f in e['fields']]
    body = ''.join(f'    /// The `{n}` component.\n    pub {n}: {t},\n' for n, t in fields)
    head = f"/// {e['doc']}\n#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]\n"
    if exhaustive:
        head += f'// exhaustive: {exhaustive}\n'
        tail = ''
    else:
        args = ', '.join(f'{n}: {t}' for n, t in fields)
        inits = ', '.join(n for n, _ in fields)
        head += '#[non_exhaustive]\n'
        tail = (f'\nimpl {name} {{\n    /// New.\n    pub const fn new({args}) -> Self {{\n'
                f'        Self {{ {inits} }}\n    }}\n}}\n')
    return f'{head}pub struct {name} {{\n{body}}}\n{tail}'


def handle_source(e, name):
    return (f"/// {e['doc']}\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]\n"
            '// exhaustive: its fields are private, so mods cannot build or destructure it\n'
            f'pub struct {name} {{\n    pub(crate) epoch: crate::WorldEpoch,\n    pub(crate) id: u64,\n}}\n\n'
            f'impl {name} {{\n    /// Create a handle for the game.\n'
            '    // hidden: a game-side helper, not part of the mod surface\n    #[doc(hidden)]\n'
            '    pub fn new(epoch: crate::WorldEpoch, id: u64) -> Self {\n        Self { epoch, id }\n    }\n}\n\n'
            f'/// What the game reads of a [`{name}`].\n#[doc(hidden)]\n#[derive(horizon_mod_api_macros::Wire)]\npub struct {name}Fields {{\n'
            '    pub epoch: crate::WorldEpoch,\n    pub id: u64,\n}\n\n'
            f'impl crate::__private::game::Fields for {name} {{\n    type Fields<\'a> = {name}Fields;\n'
            f'    fn fields(&self) -> {name}Fields {{\n        {name}Fields {{ epoch: self.epoch, id: self.id }}\n    }}\n}}\n')


def backend_source(e, name):
    base = name.removesuffix('Ref')
    side = e.get('side', 'server')
    body = ''.join(
        f"    /// Side: {side}.\n    /// {m['doc']}\n    #[access(WorldOnly)]\n"
        f"    fn {snake(base)}_{m['name']}(&self, handle: {name}) -> {m['ret']};\n" for m in e['methods'])
    return (f'/// The handles\' reads of the `{e["topic"]}` area, implemented by the game and reached through [`Backend`].\n'
            '// hidden: a handle\'s backend, reached through `Backend`; mods call the handle, never this trait\n'
            f'#[doc(hidden)]\n#[horizon_mod_api_macros::host_area]\npub trait {base}Backend {{\n{body}}}\n')


def handle_read(e, name, m):
    """The game's inherent method the generated host forwarder calls: it resolves the handle, and the read is
    filled in by hand (a stale handle reads None)."""
    if not m['ret'].startswith('Option<'):
        raise Refuse(f"{name}.{m['name']}: a handle read returns Option, None for a stale handle")
    base = snake(name.removesuffix('Ref'))
    return (f"\nimpl crate::game::Game<'_> {{\n    /// {m['doc']}\n"
            f"    pub(crate) fn {base}_{m['name']}(&self, handle: mod_api::{name}) -> {m['ret']} {{\n"
            f"        let _id = {base}_ref_to_engine(self, handle).ok()?;\n        None\n    }}\n}}\n")


def conversion_source(e, name):
    eng, leaf = e['engine'], e['engine'].split('::')[-1]
    if e['kind'] == 'handle':
        s = snake(name.removesuffix('Ref'))
        return (f'/// Names an engine `{leaf}` by its id as a handle of the open world; None outside one.\n'
                f'pub(crate) fn {s}_ref_to_api(game: &crate::game::Game<\'_>, id: u64) -> Option<mod_api::{name}> {{\n'
                f'    Some(mod_api::{name}::new(game.native_lifecycle.epoch()?, id))\n}}\n\n'
                f'/// The id a handle names, refused as stale after the world it came from closed.\n'
                f'pub(crate) fn {s}_ref_to_engine(game: &crate::game::Game<\'_>, handle: mod_api::{name}) '
                f'-> Result<u64, Stale{name}> {{\n    use mod_api::__private::game::*;\n'
                f'    (game.native_lifecycle.epoch() == Some(handle.fields().epoch))\n'
                f'        .then(|| handle.fields().id)\n        .ok_or(Stale{name})\n}}\n\n'
                f'/// A handle from a closed world.\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n'
                f'pub(crate) struct Stale{name};\n' + ''.join(handle_read(e, name, m) for m in e.get('methods', [])))
    if e['kind'] == 'enum':
        up = ''.join(f'            A::{v} => E::{v},\n' for v in e['variants'])
        down = ''.join(f'            E::{v} => A::{v},\n' for v in e['variants'])
        return (f'impl super::TryIntoApi for {eng} {{\n    type Api = mod_api::{name};\n'
                f'    type Error = std::convert::Infallible;\n'
                f'    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {{\n'
                f'        use {eng} as E;\n        use mod_api::{name} as A;\n        Ok(match self {{\n{down}        }})\n    }}\n}}\n\n'
                f'/// A mod value naming a variant this game does not know.\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n'
                f'pub(crate) struct Unsupported{name};\n\n'
                f'impl super::TryIntoEngine<()> for mod_api::{name} {{\n    type Engine = {eng};\n'
                f'    type Error = Unsupported{name};\n'
                f'    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {{\n'
                f'        use {eng} as E;\n        use mod_api::{name} as A;\n        Ok(match self {{\n{up}'
                f'            _ => return Err(Unsupported{name}),\n        }})\n    }}\n}}\n')
    fields = [split_field(f)[0] for f in e['fields']]
    reads = e.get('engine_read') or [f'self.{f}' for f in fields]  # how to read each field off the engine value
    new_args = ', '.join(reads)
    api_built = (f'mod_api::{name} {{ ' + ', '.join(f'{f}: {r}' for f, r in zip(fields, reads)) + ' }'
                 if e.get('exhaustive') else f'mod_api::{name}::new({new_args})')
    built = e['engine_new'].format(**{f: f'self.{f}' for f in fields}) if 'engine_new' in e else (
        f'{eng} {{ ' + ', '.join(f'{f}: self.{f}' for f in fields) + ' }')
    return (f'impl super::TryIntoApi for {eng} {{\n    type Api = mod_api::{name};\n'
            f'    type Error = std::convert::Infallible;\n'
            f'    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {{\n'
            f'        Ok({api_built})\n    }}\n}}\n\n'
            f'impl super::TryIntoEngine<()> for mod_api::{name} {{\n    type Engine = {eng};\n'
            f'    type Error = std::convert::Infallible;\n'
            f'    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {{\n'
            f'        Ok({built})\n    }}\n}}\n')


class Plan:
    """Pending file contents; reads lazily from disk and refuses to clobber."""

    def __init__(self, root):
        self.root, self.files, self.created = root, {}, set()

    def read(self, rel):
        if rel not in self.files:
            path = self.root / rel
            self.files[rel] = path.read_text() if path.exists() else None
            if self.files[rel] is None:
                self.created.add(rel)
        return self.files[rel]

    def append(self, rel, text, header=''):
        cur = self.read(rel)
        self.files[rel] = (header + text) if cur is None else cur.rstrip('\n') + '\n\n' + text

    def insert_after_last(self, rel, pattern, text):
        cur = self.read(rel)
        hits = list(re.finditer(pattern, cur, re.M))
        if not hits:
            raise Refuse(f'{rel}: no line matching {pattern!r} to insert after')
        at = hits[-1].end()
        self.files[rel] = cur[:at] + '\n' + text.rstrip('\n') + cur[at:]


def scaffold(entry, plan):
    check_entry(entry)
    name, topic, domain = api_name(entry), entry['topic'], entry['domain']
    if topic in ('lib', 'mod'):
        raise Refuse(f'{name}: {topic}.rs is never a target')
    declared = re.compile(rf'\b(struct|enum|trait|type)\s+({name}|{name.removesuffix("Ref")}Backend)\b')
    for p in (plan.root / API).rglob('*.rs'):
        if declared.search(p.read_text()):
            raise Refuse(f'{name}: already declared in {p.relative_to(plan.root)}')
    for rel in list(plan.files):
        if plan.files[rel] and declared.search(plan.files[rel]):
            raise Refuse(f'{name}: declared twice in this run')
    src = handle_source(entry, name) if entry['kind'] == 'handle' else mirror_source(entry, name)
    doc = f"//! {entry.get('topic_doc', topic.replace('_', ' ').capitalize() + ' values.')}\n"
    target = API / domain / f'{topic}.rs'
    plan.append(target, src, doc + '\n')
    if target in plan.created:
        plan.append(API / domain / 'mod.rs', f'pub(crate) mod {topic};\n')
        plan.created.discard(target)
    plan.append(API / 'exports' / f'{topic}.rs', f'pub use crate::{domain}::{topic}::{name};\n')
    if entry.get('methods'):
        header = '//! Host method declarations; access defaults and game forwarders are generated.\n#![allow(unused_variables)]\nuse crate::*;\n\n'
        plan.append(API / 'host' / f'{topic}.rs', backend_source(entry, name), header)
    conv = CONV / f'{topic}.rs'
    plan.append(conv, conversion_source(entry, name),
                f"//! Conversions between engine `{entry['engine'].split('::')[-1]}` and the API's `{name}`.\n\n")
    if conv in plan.created:
        plan.insert_after_last(CONV.with_suffix('.rs'), r'^pub\(in crate::game\) mod \w+;$',
                               f'#[path = "api_values/{topic}.rs"]\npub(in crate::game) mod {topic};')
        plan.created.discard(conv)


def leaves(ty):
    ty = re.sub(r"'\w+", '', ty or '')
    return {w.split('::')[-1].strip() for w in re.findall(r'[A-Za-z_]\w*(?:\s*::\s*[A-Za-z_]\w*)*', ty)} - SKIP


def available(root):
    names = set()
    for p in (root / API).rglob('*.rs'):
        text = p.read_text()
        names |= set(re.findall(r'\bpub(?:\(crate\))? (?:struct|enum|trait|type) (\w+)', text))
        for line in re.findall(r'^pub use [^;]*;', text, re.M | re.S):
            names |= set(re.findall(r'\b[A-Z]\w*', line))
    return names | {n[:-3] for n in names if n.endswith(('Ref', 'Key'))}  # engine `Mob` is carried by `MobRef`


def blockers(inventory, have):
    """target -> engine leaf type names that mod_api lacks (arguments, return, non-Game receiver)."""
    out = {}
    for x in inventory:
        s = x['raw_shape']
        if s and x['target'] not in out:
            used = set().union(*(leaves(a['ty']) for a in s['args']), leaves(s['return_ty']),
                               leaves(s['receiver']['ty'] if s['receiver'] else ''))
            out[x['target']] = used - have
    return out


def rank(blocked, resolved=()):
    """Per missing leaf: (points it alone unblocks, points it touches), best first."""
    solo, touch = Counter(), Counter()
    for miss in blocked.values():
        miss = miss - set(resolved)
        touch.update(miss)
        if len(miss) == 1:
            solo.update(miss)
    return sorted(((t, solo[t], n) for t, n in touch.items()), key=lambda r: (-r[1], -r[2], r[0]))


def report(entries, plan, blocked, root):
    leafs = {e['engine'].split('::')[-1]: e for e in entries}
    lines = ['# mod_api scaffold: dry run', '',
             f'Nothing was written. {len(entries)} allow-listed types, ranked by points each unblocks alone.', '',
             '"Alone" counts points whose only missing type is this one; "touches" counts every point using it. '
             'The census records type names without crate paths, so same-named engine types are counted together.', '']
    after = rank(blocked, leafs)
    ok_now = sum(1 for m in blocked.values() if not m)
    ok_after = sum(1 for m in blocked.values() if not (m - set(leafs)))
    lines += [f'Points with no missing type today: {ok_now} of {len(blocked)}; with every type below: {ok_after} '
              f'(+{ok_after - ok_now}).', '', '| Type | Kind | API name | Unblocks alone | Touches |', '|---|---|---|---:|---:|']
    stats = {t: (s, n) for t, s, n in rank(blocked)}
    order = sorted(entries, key=lambda e: -stats.get(e['engine'].split('::')[-1], (0, 0))[0])
    for e in order:
        s, n = stats.get(e['engine'].split('::')[-1], (0, 0))
        lines.append(f"| `{e['engine']}` | {e['kind']} | `{api_name(e)}` | {s} | {n} |")
    for e in order:
        leaf, name = e['engine'].split('::')[-1], api_name(e)
        s, n = stats.get(leaf, (0, 0))
        sample = sorted(t for t, m in blocked.items() if m == {leaf})[:4]
        lines += ['', f'## `{name}` ({e["kind"]} of `{e["engine"]}`)', '',
                  f'Unblocks {s} points alone, touches {n}. Examples: ' + ', '.join(f'`{t}`' for t in sample), '']
        one = Plan(root)
        scaffold(e, one)
        for rel in sorted(one.files):
            new = one.files[rel]
            old = (root / rel).read_text() if (root / rel).exists() else ''
            added = [l[1:] for l in difflib.unified_diff(old.splitlines(), new.splitlines(), lineterm='', n=0)
                     if l[0] == '+' and not l.startswith('+++')]
            lines += [f'`{rel}` ({"edited" if old else "created"}):', '```rust', '\n'.join(added).strip('\n'), '```']
    lines += ['', '## Next blockers not in the allow-list', '',
              'Several names (`Value`, `Pos`, `Context`, `Target`, `Hit`, `Dim`) are shared by unrelated engine types; '
              'each needs its engine path chosen before it can be allow-listed. `DVec3` and `Mat4` are glam types.', '', '| Missing type | Alone | Touches |', '|---|---:|---:|']
    lines += [f'| `{t}` | {s} | {n} |' for t, s, n in after[:25]]
    return '\n'.join(lines) + '\n'


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--config', type=Path, default=Path(__file__).with_name('api_scaffold.toml'))
    ap.add_argument('--root', type=Path, default=ROOT)
    ap.add_argument('--inventory', type=Path, help='points-inventory-runtime.json, for the ranking')
    ap.add_argument('--report', type=Path, help='write the dry-run report here')
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument('--dry-run', action='store_true')
    mode.add_argument('--apply', action='store_true')
    args = ap.parse_args(argv)
    entries = tomllib.loads(args.config.read_text())['type']
    plan = Plan(args.root)
    try:
        for e in entries:
            scaffold(e, plan)
    except Refuse as err:
        sys.exit(f'refused: {err}')
    if args.apply:
        for rel, text in plan.files.items():
            (args.root / rel).parent.mkdir(parents=True, exist_ok=True)
            (args.root / rel).write_text(text)
        print(f'wrote {len(plan.files)} files')
        return
    import json
    blocked = blockers(json.loads(args.inventory.read_text()), available(args.root)) if args.inventory else {}
    text = report(entries, plan, blocked, args.root)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(text)
    else:
        print(text)


if __name__ == '__main__':
    main()
