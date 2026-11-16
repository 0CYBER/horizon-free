"""Source-only Wire declaration guard (no dependence on Cargo OUT_DIR caches).

Generator templates are checked before interpolation, so adding another event or
provider cannot silently bypass the rule. Keep API_TEMPLATE_FILES aligned with
horizon-mod-api-gen's API output writers when introducing a new output writer.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
API_TEMPLATE_FILES = (
    "lib.rs", "host.rs", "dispatch.rs", "capability_providers.rs", "wire/calls.rs",
)
# Strings/comments are tokens so doc examples and quoted test programs never
# become declarations. Raw Rust strings and nested block comments are supported.
TOKEN = re.compile(r'r(?P<hashes>\#*)".*?"(?P=hashes)|"(?:\\.|[^"\\])*"|//[^\n]*|/\*|\*/|\b\w+\b|[^\s]', re.S)


def tokens(source):
    result, depth = [], 0
    for m in TOKEN.finditer(source):
        value = m.group()
        if value == "/*":
            depth += 1
        elif value == "*/" and depth:
            depth -= 1
        elif not depth:
            result.append((value, m.start()))
    return result


def cfg_requires_test(values):
    """True only if the cfg predicate cannot hold with test disabled.

    Other configuration atoms are unknown; any(test, feature) and not(test)
    therefore remain production code. Nested all/any/not are evaluated.
    """
    def possible(parts):
        if parts == ['test']:
            return {False}
        if len(parts) >= 3 and parts[0] in ('all', 'any', 'not') and parts[1] == '(' and parts[-1] == ')':
            args, start, depth = [], 2, 0
            for j in range(2, len(parts) - 1):
                depth += (parts[j] == '(') - (parts[j] == ')')
                if parts[j] == ',' and not depth:
                    if j > start:
                        args.append(possible(parts[start:j]))
                    start = j + 1
            if start < len(parts) - 1:
                args.append(possible(parts[start:-1]))
            if parts[0] == 'not':
                return {not v for v in args[0]} if len(args) == 1 else {True, False}
            outcomes = {parts[0] == 'all'}
            for arg in args:
                outcomes = {(a and b) if parts[0] == 'all' else (a or b)
                            for a in outcomes for b in arg}
            return outcomes
        return {True, False}
    return True not in possible(values)


def code_tokens(source):
    """Remove examples and test-only modules/functions, retaining attributes."""
    ts = [(v, p) for v, p in tokens(source) if not v.startswith(('//', '"', 'r"', 'r#'))]
    out, i = [], 0
    while i < len(ts):
        end = i + 2
        if [v for v, _ in ts[i:i + 4]] == ['#', '[', 'cfg', '(']:
            while end < len(ts) and ts[end][0] != ']':
                end += 1
        else:
            end = i
        if end > i and cfg_requires_test([v for v, _ in ts[i + 4:end - 1]]):
            i = end + 1
            while i < len(ts) and ts[i][0] not in ('{', ';'):
                i += 1
            if i < len(ts) and ts[i][0] == '{':
                depth = 1
                i += 1
                while i < len(ts) and depth:
                    depth += (ts[i][0] == '{') - (ts[i][0] == '}')
                    i += 1
            else:
                i += 1
        else:
            out.append(ts[i])
            i += 1
    return out


def violations(source):
    ts = code_tokens(source)
    missing = []
    for i, (value, pos) in enumerate(ts):
        if value != 'pub' or i + 2 >= len(ts) or ts[i + 1][0] not in ('struct', 'enum', 'type'):
            continue
        name = ts[i + 2][0]
        if name in ('$', '{', '#'):
            name += ts[i + 3][0]  # macro or format/quote template placeholder
        # Only the attached attribute/comment block can justify a declaration.
        start = pos
        j = i - 1
        while j >= 0 and ts[j][0] == ']':
            depth = 1
            j -= 1
            while j >= 0 and depth:
                depth += (ts[j][0] == ']') - (ts[j][0] == '[')
                j -= 1
            if j >= 0 and ts[j][0] == '#':
                start = ts[j][1]
                j -= 1
            else:
                break
        line_start = source.rfind('\n', 0, start) + 1
        prefix = source[line_start:start]
        before = source[:line_start].splitlines()
        while before and before[-1].strip().startswith('//'):
            prefix = before.pop() + '\n' + prefix
        attrs = source[start:pos]
        derive = any(re.search(r'(?:^|[,\s])(?:\w+::)*Wire(?:\s|,|$)', a)
                     for a in re.findall(r'#\s*\[\s*derive\s*\((.*?)\)\s*\]', attrs, re.S))
        reason = re.search(r'(?m)^[ \t]*// no wire:[ \t]*\S[^\n]*', prefix + attrs)
        if not (derive or reason):
            missing.append((source.count('\n', 0, pos) + 1, name))
    return missing


def decode_literal(literal):
    if literal.startswith('"'):
        return literal[1:-1].replace('\\n', '\n').replace('\\"', '"')
    if re.match(r'r#*"', literal):
        return literal.split('"', 1)[1].rsplit('"', 1)[0]
    return None


def template_violations(source):
    """Check literal templates, resolving only proven literal policy fragments.

    An if/else binding qualifies only when BOTH literal arms explicitly satisfy
    the policy. Unknown expressions, reassignment and out-of-scope bindings fail
    closed. No variable name (including wire_form) gets special treatment.
    """
    ts = tokens(source)
    scopes, stack = {}, []
    for value, pos in ts:
        scopes[pos] = tuple(stack)
        if value == '{':
            stack.append(pos)
        elif value == '}' and stack:
            stack.pop()
    literal_pattern = r'"(?:\\.|[^"\\])*"'
    binding_pattern = re.compile(
        r'\blet\s+(\w+)\s*=\s*if\s+[^{};]+\{\s*(' + literal_pattern
        + r')\s*\}\s*else\s*\{\s*(' + literal_pattern + r')\s*\}\s*;', re.S)
    bindings = []
    for match in binding_pattern.finditer(source):
        # Ignore matches inside comments or string literals.
        if match.start() not in scopes:
            continue
        forms = [decode_literal(match[n]) for n in (2, 3)]
        if all(not violations(form + '\npub struct __PolicyProbe;') for form in forms):
            bindings.append(match)
    result = []
    for literal, pos in ts:
        template = decode_literal(literal)
        if template is None:
            continue
        for binding in bindings:
            if binding.end() > pos:
                continue
            scope = scopes[binding.start()]
            if scopes[pos][:len(scope)] != scope:
                continue
            name = binding[1]
            if re.search(r'\b' + re.escape(name) + r'\s*=', source[binding.end():pos]):
                continue
            # Both arms proved valid; use the first for policy inspection only.
            template = template.replace('{' + name + '}', decode_literal(binding[2]) + '\n')
        result.extend((source.count('\n', 0, pos) + 1, line, name)
                      for line, name in violations(template))
    return result


def runtime_reexport_names(source):
    """Follow public use trees, including groups and aliases, into the runtime."""
    ts = [value for value, _ in code_tokens(source)]
    names = set()

    def walk(parts, prefix=()):
        i = 0
        while i < len(parts):
            path = list(prefix)
            grouped = False
            while i < len(parts) and parts[i] != ',':
                value = parts[i]
                i += 1
                if value == '{':
                    start, depth = i, 1
                    while i < len(parts) and depth:
                        depth += (parts[i] == '{') - (parts[i] == '}')
                        i += 1
                    walk(parts[start:i - 1], tuple(path))
                    grouped = True
                elif value == 'as':
                    i += 1  # The exported alias does not rename its definition.
                elif value != ':':
                    path.append(value)
            if not grouped and path and path[0] == 'horizon_mod_runtime':
                names.add(path[-1])
            i += 1

    for i in range(len(ts) - 1):
        if ts[i:i + 2] == ['pub', 'use']:
            end = i + 2
            while end < len(ts) and ts[end] != ';':
                end += 1
            walk(ts[i + 2:end])
    return names


def repository_violations(root=ROOT):
    sources = {}
    for crate in ('horizon-mod-api', 'horizon-mod-vocab'):
        api = root / 'crates' / crate / 'src'
        for path in sorted(api.rglob('*.rs')):
            if path.name == '__private.rs' or path.name.endswith('_tests.rs'):
                continue
            sources[str(path.relative_to(root))] = path.read_text()
    result = []
    for path, source in sources.items():
        result.extend(f'{path}:{line}: {name}' for line, name in violations(source))
    # Runtime-owned types are public API too; policy lives on their definition,
    # not on a use alias or on an unrelated hand-written impl with the same name.
    runtime_names = set().union(*(runtime_reexport_names(s) for s in sources.values()))
    if '*' in runtime_names:
        result.append('runtime re-export: wildcard requires explicit named exports for Wire policy')
    runtime = root / 'crates/horizon-mod-runtime/src'
    for path in sorted(runtime.rglob('*.rs')):
        result.extend(f'{path.relative_to(root)}:{line}: {name}'
                      for line, name in violations(path.read_text()) if name in runtime_names)
    gen = root / 'crates/horizon-mod-api-gen/src'
    for relative in API_TEMPLATE_FILES:
        path = gen / relative
        source = path.read_text()
        # Only production string literals, never Rust generator implementation
        # types or cfg(test) fixture programs, are emitted API templates.
        production = source.split('#[cfg(test)]', 1)[0]
        for source_line, line, name in template_violations(production):
            result.append(f'{path.relative_to(root)}:{source_line}: template line {line}: {name}')
    return result


if __name__ == '__main__':
    failures = repository_violations()
    for failure in failures:
        print(f'{failure}: missing Wire policy; add #[derive(mod_api::Wire)] or document an opt-out with // no wire: <reason>.')
    raise SystemExit(bool(failures))
