"""Discover recursive public vocabulary types, including indirect cycles.

Uses the Wire declaration guard's Rust lexer so comments, strings and test-only
items cannot introduce phantom types. Run directly to print the inventory.
"""
from pathlib import Path
import re
from check_mod_api_wire import ROOT, code_tokens


def recursive_types(root=ROOT / "crates/horizon-mod-vocab/src", extra_paths=()):
    declarations = {}
    external_json = set()
    for path in sorted([*Path(root).rglob("*.rs"), *extra_paths]):
        ts = [value for value, _ in code_tokens(path.read_text())]
        # Handwritten external JSON codecs are recursive even though their
        # declarations live in a dependency, not the public vocabulary source.
        external_json.update("serde_json::" + name for name in re.findall(
            r"impl (?:crate : : wire : : )?Wire for serde_json : : (\w+)", " ".join(ts)))
        for i in range(len(ts) - 3):
            if ts[i:i + 2] not in (["pub", "struct"], ["pub", "enum"], ["pub", "type"]):
                continue
            name = ts[i + 2]
            j = i + 3
            angle = 0
            while j < len(ts):
                if not angle and ts[j] in ("{", "(", ";", "="):
                    break
                angle += (ts[j] == "<") - (ts[j] == ">")
                j += 1
            if j == len(ts) or ts[j] == ";":
                continue
            start = j
            if ts[j] == "=":
                while j < len(ts) and ts[j] != ";":
                    j += 1
            else:
                closing = "}" if ts[j] == "{" else ")"
                opening = ts[j]
                depth = 1
                j += 1
                while j < len(ts) and depth:
                    depth += (ts[j] == opening) - (ts[j] == closing)
                    j += 1
            parts = ts[start:j]
            # Variant names and named-field labels are not type references.
            excluded = set()
            depth = 0
            for k, token in enumerate(parts):
                if token == "{" or token == "(":
                    depth += 1
                elif token == "}" or token == ")":
                    depth -= 1
                elif ts[i + 1] == "enum" and depth == 1 and (k == 1 or parts[k - 1] == ","):
                    excluded.add(k)
                if k + 1 < len(parts) and parts[k + 1] == ":" and (k + 2 == len(parts) or parts[k + 2] != ":"):
                    excluded.add(k)
                # External qualified types with coincidentally identical names.
                if k >= 3 and parts[k - 2:k] == [":", ":"] and parts[k - 3] in ("runtime", "std", "horizon_mod_runtime"):
                    excluded.add(k)
            body = {v for k, v in enumerate(parts) if k not in excluded}
            if "Self" in body:
                body.add(name)
            declarations.setdefault(name, set()).update(body)
    graph = {name: refs & declarations.keys() for name, refs in declarations.items()}
    result = []
    for name in sorted(graph):
        pending, seen = list(graph[name]), set()
        while pending:
            current = pending.pop()
            if current == name:
                result.append(name)
                break
            if current not in seen:
                seen.add(current)
                pending.extend(graph[current])
    return sorted(set(result) | external_json)


if __name__ == "__main__":
    print("\n".join(recursive_types(extra_paths=[ROOT / "crates/horizon-core/src/nbt.rs"])))
