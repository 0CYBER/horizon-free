#!/usr/bin/env python3
"""Structural diff between vanilla's exported command tree and Horizon's.

usage: command_tree_diff.py VANILLA_TREE.json RUST_TREE.json [ROOT]

Reports, for every shared node, the children Horizon lacks or adds, differing
argument parsers and executable/redirect flags. Missing subtrees are reported
once at their root.
"""
import json
import sys

LOOSE = {
    # Horizon's coarse argument kinds that stand in for several vanilla parsers.
    'brigadier:string': {'brigadier:string', 'minecraft:message', 'minecraft:component'},
}


def same_parser(vanilla, rust):
    if vanilla == rust:
        return True
    return False


def keyed(children):
    """Literals by name; arguments by position (their names are internal)."""
    out = {}
    index = 0
    for name, child in children.items():
        if child['type'] == 'literal':
            out[('l', name)] = (name, child)
        else:
            out[('a', index)] = (name, child)
            index += 1
    return out


def diff(path, v, r, out):
    vc, rc = keyed(v.get('children', {})), keyed(r.get('children', {}))
    for key, (name, node) in vc.items():
        if key not in rc:
            out.append(('missing', path + [name], node.get('parser') or 'literal', count(node)))
    for key, (name, node) in rc.items():
        if key not in vc:
            out.append(('extra', path + [name], node.get('parser') or 'literal', count(node)))
    for key, (name, a) in vc.items():
        if key not in rc:
            continue
        _, b = rc[key]
        if a['type'] == 'argument' and not same_parser(a['parser'], b['parser']):
            out.append(('parser', path + [name], f"{a['parser']} vs {b['parser']}", 0))
        if bool(a.get('executable')) != bool(b.get('executable')):
            out.append(('executable', path + [name], f"vanilla={bool(a.get('executable'))}", 0))
        if bool(a.get('redirect')) != bool(b.get('redirect')):
            out.append(('redirect', path + [name], f"vanilla={bool(a.get('redirect'))}", 0))
        diff(path + [name], a, b, out)


def count(node):
    return 1 + sum(count(c) for c in node.get('children', {}).values())


def main():
    vanilla = json.load(open(sys.argv[1]))
    rust = json.load(open(sys.argv[2]))
    only = sys.argv[3] if len(sys.argv) > 3 else None
    if only:
        vanilla = {'children': {only: vanilla['children'][only]}, 'type': 'root'}
        rust = {'children': {only: rust['children'].get(only, {'children': {}, 'type': 'literal'})}, 'type': 'root'}
    out = []
    diff([], vanilla, rust, out)
    for kind, path, detail, size in out:
        print(f'{kind:10} {" ".join(path)}  [{detail}]' + (f' ({size} nodes)' if size else ''))
    missing = sum(size for kind, _, _, size in out if kind == 'missing')
    print(f'-- {len(out)} differences, {missing} vanilla nodes missing in Horizon')


if __name__ == '__main__':
    main()
