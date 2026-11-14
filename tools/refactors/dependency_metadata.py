#!/usr/bin/env python3
"""Migrate readonly dependency constructor fixtures to the complete declaration shape."""
from pathlib import Path

for path in [Path('crates/horizon-mod-vocab/src/world/phase.rs'), Path('crates/horizon-mod-vocab/src/wire/wire_types.rs'), Path('crates/horizon-mod-vocab/src/wire/wire_layout_tests.rs')]:
    source = path.read_text()
    needle = 'PackageDependency::new('
    cursor = 0
    replacements = []
    while (start := source.find(needle, cursor)) >= 0:
        begin = start + len(needle)
        depth, quote, escape = 0, None, False
        parts, part = [], begin
        for end in range(begin, len(source)):
            char = source[end]
            if quote:
                if escape:
                    escape = False
                elif char == '\\':
                    escape = True
                elif char == quote:
                    quote = None
                continue
            if char == '"':
                quote = char
            elif char in '([{':
                depth += 1
            elif char == ')' and depth == 0:
                if source[part:end].strip():
                    parts.append(source[part:end].strip())
                break
            elif char in ')]}':
                depth -= 1
            elif char == ',' and depth == 0:
                parts.append(source[part:end].strip())
                part = end + 1
        assert len(parts) == 3 and parts[2] in ('true', 'false'), (path, parts)
        kind = 'Optional' if parts[2] == 'true' else 'Required'
        args = parts[:2] + [f'crate::DependencyKind::{kind}', 'crate::DependencyOrder::After', 'crate::Side::Both', 'None', 'None']
        replacements.append((begin, end, ', '.join(args)))
        cursor = end + 1
    for begin, end, value in reversed(replacements):
        source = source[:begin] + value + source[end:]
    path.write_text(source)
