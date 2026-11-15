#!/usr/bin/env python3
"""Require explicit ownership reasons for vocabulary process-global declarations.

The allowlist is a reviewed inventory, not a discovery-generated permission list.
Comments, literals and unconditional unit-test items do not declare production
state. A feature-gated declaration is checked even when its feature is off.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

from layering.rust_text import blank, item_end

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/horizon-mod-vocab/src"
INVENTORY = ROOT / "tools/layering/mod_vocab_state.json"
DOCUMENT = ROOT / "docs/layering/mod-vocab-state.md"
STATE = re.compile(r"(?<![\w'])static\s+(?:(?:mut|ref)\s+)?(?P<name>[A-Za-z_]\w*)\s*:(?!:)\s*(?P<type>[^=]+?)=")
TEST = re.compile(r"#\[\s*(?:test\s*|cfg\s*\(\s*test\s*\)\s*)\]")


def declarations(source: str) -> list[tuple[str, str, int]]:
    """Return name, normalized type and line for each non-test static/TLS slot."""
    code = blank(source)
    prefix = re.match(r"\s*(?:#!\[[^\]]*\]\s*)*", code).group()
    if re.search(r"#!\[\s*cfg\s*\(\s*test\s*\)\s*\]", prefix):
        return []
    regions = [(m.start(), item_end(code, m.end())[0]) for m in TEST.finditer(code)]
    functions = [(m.start(), item_end(code, m.start())[0], m[1])
                 for m in re.finditer(r"\bfn\s+(\w+)", code)]
    found = []
    for match in STATE.finditer(code):
        if any(start <= match.start() < stop for start, stop in regions):
            continue
        owners = [name for start, stop, name in functions if start <= match.start() < stop]
        name = '::'.join([*owners, match['name']])
        found.append((name, re.sub(r"\s+", "", match['type']), code.count('\n', 0, match.start()) + 1))
    return found


def test_only_files(files: dict[str, str]) -> set[str]:
    """Follow explicit Rust module edges, never treating a filename as an exemption."""
    edges = []
    pattern = re.compile(r"(?P<attrs>(?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?\bmod\s+(?P<name>\w+)\s*;")
    for path, source in files.items():
        code = blank(source)
        for match in pattern.finditer(code):
            original = source[match.start():match.end()]
            explicit = re.search(r'#\[path\s*=\s*"([^"\n]+)"\s*\]', original)
            owner = Path(path)
            directory = owner.parent if owner.name in ('lib.rs', 'mod.rs') else owner.with_suffix('')
            child = owner.parent / explicit[1] if explicit else directory / (match['name'] + '.rs')
            edges.append((path, child.as_posix(), bool(re.search(r'#\[cfg\(test\)\]', match['attrs']))))
    only = set()
    while True:
        testing = {child for parent, child, test in edges if test or parent in only}
        production = {child for parent, child, test in edges if not test and parent not in only}
        updated = testing - production
        if updated == only:
            return only
        only = updated


def unsafe_violations(files: dict[str, str]) -> list[str]:
    """The unchanged points machinery is the sole temporary source exception."""
    errors = []
    if not re.search(r'#!\[deny\(unsafe_code\)\]', blank(files.get('lib.rs', ''))):
        errors.append('lib.rs must retain #![deny(unsafe_code)] until the points follow-up installs forbid')
    for path, source in files.items():
        if path == '__private/points.rs':
            continue
        code = blank(source)
        if re.search(r'\bunsafe\b', code):
            errors.append(f'{path}: unsafe is permitted only in the pending points split')
        if re.search(r'\b(?:allow|expect)\s*\([^)]*\bunsafe_code\b', code):
            errors.append(f'{path}: unsafe_code suppression is permitted only in __private/points.rs')
    return errors


def symlink_violations(source_root: Path) -> list[str]:
    """Do not let Rust descend a directory that the source scan never visits."""
    return [f'{path.relative_to(source_root)}: symlinks are refused because descendant modules can escape the scanned source tree'
            for path in source_root.rglob('*') if path.is_symlink()]


def expansion_violations(files: dict[str, str], source_root: Path = SOURCE) -> list[str]:
    """Refuse state-producing macro definitions and unchecked include boundaries.

    Listed OUT_DIR inputs pass the generator's token-level vocabulary validator
    before every write (build/vocab_boundary.rs), including unsafe/suppressions.
    """
    generated = {
        'mod_api_generated_host.rs', 'mod_api_generated.rs', 'mod_api_exports_root.rs',
        'wire_generated.rs', 'parts_modules.rs', 'mod_api_host_areas.rs',
        'capability_providers_generated.rs', 'mod_api_exports_private_reload.rs',
        'mod_api_exports_private_built.rs', 'mod_api_exports_private_game.rs',
        'mod_api_exports_private.rs',
    }
    errors = []
    scanned = {(source_root / path).resolve() for path in files}
    for path, source in files.items():
        code = blank(source)
        if not (source_root / path).resolve().is_relative_to(source_root.resolve()):
            errors.append(f'{path}: source symlink leaves the vocabulary source root')
        if Path(path).parent.name != 'exports':
            modules = re.compile(r'(?P<attrs>(?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?\bmod\s+(?P<name>\w+)\s*;')
            owner = Path(path)
            directory = owner.parent if owner.name in ('lib.rs', 'mod.rs') else owner.with_suffix('')
            for module in modules.finditer(code):
                if re.search(r'#\[\s*path\s*=', module['attrs']):
                    continue
                choices = [directory / (module['name'] + '.rs'), directory / module['name'] / 'mod.rs']
                if not any((source_root / child).resolve() in scanned for child in choices):
                    errors.append(f'{path}: module {module["name"]} targets source outside the scanned vocabulary set')
        inline_modules = [(module.start(), item_end(code, module.start())[0])
                          for module in re.finditer(r'(?<![\w#])\bmod\b[^;{]*\{', code)]
        for attr in re.finditer(r'#\[\s*path\s*=', code):
            if any(start < attr.start() < end for start, end in inline_modules):
                errors.append(f'{path}: nested inline-module path attributes are refused because Rust resolves them from the inline module directory')
                continue
            literal = re.match(r'#\[\s*path\s*=\s*"([^"\n]+)"\s*\]', source[attr.start():])
            target = (source_root / Path(path).parent / literal[1]).resolve() if literal else None
            if target not in scanned:
                errors.append(f'{path}: path module target is outside the scanned vocabulary source set')
        if re.search(r'#\[\s*cfg_attr\b[^\]]*\bpath\s*=', code):
            errors.append(f'{path}: conditional path module targets are not supported by the state gate')
        tests = [(m.start(), item_end(code, m.end())[0]) for m in TEST.finditer(code)]
        declared = [(m.start(), m.end()) for m in STATE.finditer(code)]
        for token in re.finditer(r"(?<!')\bstatic\b", code):
            if not any(start <= token.start() < end for start, end in declared + tests):
                errors.append(f'{path}: unsupported static syntax or macro argument requires an explicit declaration')
        for macro in re.finditer(r'\bmacro_rules\s*!', code):
            end = item_end(code, macro.start())[0]
            body = code[macro.start():end]
            if re.search(r"(?<!')\bstatic\b", body):
                errors.append(f'{path}: state-declaring macro definitions are not permitted')
        for include in re.finditer(r'\binclude\s*!', code):
            if any(start <= include.start() < end for start, end in tests):
                continue
            end = item_end(code, include.start())[0]
            call = source[include.start():end]
            name = re.fullmatch(
                r'include!\s*\(\s*concat!\s*\(\s*env!\("OUT_DIR"\)\s*,\s*"/([^"\n]+)"\s*\)\s*\)\s*;?', call)
            if name is None or name[1] not in generated:
                errors.append(f'{path}: include boundary is not a validated vocabulary generator output')
    return errors


def violations(files: dict[str, str], allowed: dict[str, dict]) -> list[str]:
    errors = []
    seen = set()
    test_files = test_only_files(files)
    for path, source in sorted(files.items()):
        if path in test_files:
            continue
        for name, kind, line in declarations(source):
            key = f"{path}::{name}"
            entry = allowed.get(key)
            if key in seen:
                errors.append(f"{path}:{line}: duplicate inventory identity {name}; use distinct state names")
            seen.add(key)
            if entry is None:
                errors.append(f"{path}:{line}: unlisted process state {name}: {kind}")
            elif kind != entry['type']:
                errors.append(f"{path}:{line}: {name} changed type: {entry['type']} -> {kind}")
            if entry is not None and entry.get('category') == 'test-support':
                code = blank(source)
                declaration = next(match for match in STATE.finditer(code)
                                   if match['name'] == name.split('::')[-1]
                                   and code.count('\n', 0, match.start()) + 1 == line)
                attributes = re.search(r'(?:#\[[^\]]*\]\s*)+(?:pub(?:\([^)]*\))?\s+)?$', code[:declaration.start()])
                predicates = [source[attributes.start() + cfg.start(1):attributes.start() + cfg.end(1)]
                              for cfg in re.finditer(r'#\[\s*cfg\s*\(([^\]]*)\)\s*\]',
                                                     code[attributes.start():attributes.end()])] if attributes else []
                guarded = any(re.sub(r'\s+', '', predicate) in {
                    'test', 'feature="testing"', 'any(test,feature="testing")',
                    'any(feature="testing",test)',
                } for predicate in predicates)
                if not guarded:
                    errors.append(f'{path}:{line}: test-support state requires a direct test/testing-only cfg')
    for key in sorted({key for key in allowed if not key.startswith('@generated/')} - seen):
        errors.append(f"stale state inventory entry: {key}")
    return errors


def inventory_document(allowed: dict[str, dict]) -> str:
    heading = '''# Vocabulary process state and the temporary points exception

<!-- Generated by tools/check_mod_vocabulary_state.py --write-doc from tools/layering/mod_vocab_state.json. -->

Part of #2587. The vocabulary is **not stateless**, and this first change does **not** claim it is
free of unsafe. Its crate-level `deny(unsafe_code)` remains in force. The unchanged
`src/__private/points.rs` contains the sole temporary unsafe exception (local lint expectations)
and the open injection-dispatch registry. #2697 moves the final seal into
`horizon-engine-slots`; #3523 also moves generated point records and activity there, behind a set-once dispatch socket. The #2587 follow-up then moves raw-pointer
conversion and ownership behind the points boundary and installs `forbid(unsafe_code)`.

Crash detail buffers, profiling, borrowed resource scopes, static-string storage, retained fluids,
package/phase/registration snapshots, live authoring requests, concrete Part registrars and diagnostic
warning history belong to `horizon-mod-api`. The engine interface retains dispatch routing, enabled
bits, installed/per-game lookup tables, set-once callback sockets, one crash-owner `Cell`, and the
named immutable caches below. Ordinary installed key and Part reads never call an authoring service.
Registration callbacks are consulted only while the API-owned activity counter is nonzero.

Every production static/TLS declaration, including declarations behind the `testing` feature, is
listed below. The `test-support` category requires a direct `cfg(test)`, `cfg(feature = "testing")`,
or their two-way `cfg(any(...))`; broader or unrecognized predicates are refused. Its entries hold
set-once installed test metadata, never live authoring requests, and are absent from production builds. Unconditional unit-test items and explicitly test-only module files are excluded.
CI rejects an unlisted declaration, a changed type, a stale entry, unsafe source outside the points
exception, or a lint suppression outside that file. State-declaring macro definitions and unchecked
include boundaries are refused. Path modules must target the scanned vocabulary source set;
generated module declarations are checked against its canonical file paths as well. Source symlinks
are refused, including directory links that recursive scans do not enter. Path attributes nested
inside inline modules are refused because Rust resolves them from an inline-module directory, not
the file's directory. Conditional path routing and module-declaring generated macros are refused
instead of being skipped.
Every listed generated OUT_DIR input is token-checked before write by
`horizon-mod-api-gen`: no unlisted static declarations (including macro metavariables), unsafe,
unsafe lint suppression, or further includes. The generated immutable ABI hash is checked against
this same inventory by exact name and type; nested state is refused. Lifetimes and string literals are not declarations. The trusted
Wire derive supplies codecs, not state; the crate's unsafe lint applies to its expansion as well.
Ownership reasons are reviewed permissions,
not generated from newly discovered declarations. The table is generated from those permissions.

Source paths below are relative to `crates/horizon-mod-vocab/src`; `@generated/` identifies an
OUT_DIR file checked by the generator before writing it.

| File and item | Type | Ownership | Reason |
| --- | --- | --- | --- |
'''
    rows = [f"| `{key}` | `{entry['type']}` | {entry['category']} | {entry['reason']} |"
            for key, entry in sorted(allowed.items())]
    return heading + '\n'.join(rows) + '\n'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=SOURCE)
    parser.add_argument('--inventory', type=Path, default=INVENTORY)
    parser.add_argument('--write-doc', action='store_true', help='refresh the generated ownership inventory page')
    args = parser.parse_args()
    allowed = json.loads(args.inventory.read_text())
    for name, entry in allowed.items():
        if not entry.get('reason') or not entry.get('category'):
            parser.error(f"{name} needs an ownership category and reason")
    links = symlink_violations(args.source)
    if links:
        for error in links:
            print(error)
        return 1
    files = {p.relative_to(args.source).as_posix(): p.read_text() for p in args.source.rglob('*.rs')}
    errors = violations(files, allowed) + unsafe_violations(files) + expansion_violations(files, args.source)
    expected_doc = inventory_document(allowed)
    if args.write_doc:
        DOCUMENT.write_text(expected_doc)
    elif not DOCUMENT.exists() or DOCUMENT.read_text() != expected_doc:
        errors.append('ownership inventory page is stale; run tools/check_mod_vocabulary_state.py --write-doc')
    for error in errors:
        print(error)
    return bool(errors)


if __name__ == '__main__':
    raise SystemExit(main())
