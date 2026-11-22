#!/usr/bin/env python3
"""Check that each crate depends only on the layers its own layer may use (docs/layers.md).

tools/layers.json's `may_depend` says which: engine on engine; platform on engine and platform; horizon on
platform and horizon only, so the Horizon layer builds against the platform's public API and can live in its own
repository. `tool` crates (dev tools) may use anything and nothing shipped uses them. `crates` gives every crate its
layer and `allowed` lists today's exceptions. `optional` lists exceptions that hold only while the dependency is
optional (behind a feature), for the layer and the portable check alike. The check fails on:
- a workspace crate with no layer;
- a normal, build or optional dependency on a layer its crate's layer may not use that `allowed` doesn't list,
  and that isn't an optional dependency listed in `optional`;
- an `optional` entry whose dependency is no longer optional;
- with --base, an `allowed` or `optional` entry the branch added from a crate that already had a layer on the base;
- a shipped crate depending on a tool;
- a portable crate depending on a crate of the repository that isn't portable, or on a `forbidden` library;
- a `portable.pending` crate doing the same beyond the crates its entry lists, listed as portable too, or (with
  --base) gaining an entry its base list lacks.
An `allowed` or `optional` entry with no dependency left is reported, not failed. Dev-dependencies are exempt: tests may reach the platform, but shipped code may not.

    python3 tools/check_layers.py                     # the check
    python3 tools/check_layers.py --base origin/main  # and no new exceptions since the base
    python3 tools/check_layers.py --list              # each crate's layer and the exceptions left
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / "tools" / "layers.json"
CONFIG_PATH = "tools/layers.json"
TOOL = "tool"
DOC = "docs/layers.md"
# The section of docs/layers.md that says what a layer may depend on.
SECTION = {"engine": "#engine", "platform": "#platform", "horizon": "#horizon", TOOL: "#tools"}
WHERE = "#where-a-new-feature-goes"


def load_config(path=CONFIG):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def base_config(base):
    """tools/layers.json where the branch left `base` (their merge base), or None when there is none. On `base` itself,
    the exceptions main dropped since then would look like ones this branch added."""
    merge_base = subprocess.run(["git", "merge-base", base, "HEAD"], capture_output=True, text=True, cwd=ROOT)
    if merge_base.returncode != 0:
        return None
    shown = subprocess.run(["git", "show", f"{merge_base.stdout.strip()}:{CONFIG_PATH}"], capture_output=True,
                           text=True, cwd=ROOT)
    return json.loads(shown.stdout) if shown.returncode == 0 else None


def metadata(manifest, known=None):
    """Offline declarations, resolving only possible registry-to-workspace edges with an unchanged lockfile.

    `known` is the set of every local crate across all checked workspaces: a patch in one workspace may point at a
    crate that belongs to another (root, launcher/, crates/horizon-telemetry-server/...), so filtering candidates by
    this manifest's members alone would let that crossing slip past the layer check. It defaults to this workspace's
    members for direct callers with no wider config.
    """
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path", str(manifest)],
        check=True, capture_output=True, text=True, cwd=ROOT,
    ).stdout
    packages = json.loads(out)["packages"]
    if known is None:
        known = local_packages(packages)
    candidates = [(package, dep) for package in packages for dep in package["dependencies"]
                  if not dep.get("path") and dep.get("kind") != "dev" and dep["name"] in known]
    if candidates:
        # --no-deps cannot tell a patched member from an external package with the same name.
        # All features include optional edges; no target filter keeps every target's declarations.
        out = subprocess.run(
            ["cargo", "metadata", "--all-features", "--locked", "--offline", "--format-version", "1",
             "--manifest-path", str(manifest)],
            check=True, capture_output=True, text=True, cwd=ROOT,
        ).stdout
        resolve_registry(candidates, json.loads(out))
    return packages


def local_packages(packages):
    """The package names cargo metadata reported for this workspace (with --no-deps, its members)."""
    return {package["name"] for package in packages}


def resolve_registry(candidates, resolved):
    """Mark only declarations Cargo resolved to a path package, not a same-name registry/git package."""
    packages = {package["id"]: package for package in resolved["packages"]}
    nodes = {node["id"]: node for node in resolved["resolve"]["nodes"]}
    for package, dep in candidates:
        targets = set()
        for edge in nodes[package["id"]]["deps"]:
            # Cargo's edge name is the extern crate (lib target) name, which differs from the package name when a
            # crate sets a custom [lib] name. Match the resolved package's real name instead of `edge['name']`, so
            # an unrenamed dependency on such a crate still resolves. A rename, when present, is the import alias
            # and is checked against edge['name'] to disambiguate same-named packages.
            if packages[edge["pkg"]]["name"] != dep["name"]:
                continue
            if dep.get("rename") and dep["rename"] != dep["name"] \
                    and edge["name"] != dep["rename"].replace("-", "_"):
                continue
            if any(kind["kind"] == dep.get("kind") and kind["target"] == dep.get("target")
                   for kind in edge["dep_kinds"]):
                targets.add(edge["pkg"])
        if len(targets) != 1:
            raise ValueError(f"Cargo did not uniquely resolve {package['name']} -> {dep['name']}")
        target = packages[targets.pop()]
        if target["source"] is None:
            dep["path"] = str(Path(target["manifest_path"]).parent)


def resolve_local(dep):
    """Cargo's `name` is the real package name; `rename` is only the dependency's import alias."""
    return bool(dep.get("path")), dep["name"]


def edges(packages):
    """{crate: {dependency, ...}} for normal and build dependencies on other local crates.

    Includes a registry declaration resolved to a workspace crate by `[patch.crates-io]`, even though
    --no-deps reports it without a `path`.
    """
    graph = {}
    for package in packages:
        deps = graph.setdefault(package["name"], set())
        for dep in package["dependencies"]:
            if dep.get("kind") == "dev":
                continue
            local, name = resolve_local(dep)
            if local:
                deps.add(name)
    return graph


def optional_edges(packages):
    """The local dependencies, as 'crate -> dependency', that are optional everywhere they are declared (normal and
    build, every target): one mandatory declaration makes the edge mandatory."""
    declared, mandatory = set(), set()
    for package in packages:
        for dep in package["dependencies"]:
            if dep.get("kind") == "dev":
                continue
            local, name = resolve_local(dep)
            if local:
                edge = f"{package['name']} -> {name}"
                declared.add(edge)
                if not dep.get("optional"):
                    mandatory.add(edge)
    return declared - mandatory


def optional_rows(config):
    """The edges of `optional`: exceptions that hold only while the dependency stays optional."""
    return {row["edge"] for row in config.get("optional", [])}


def crossings(config, graph):
    """The dependencies on a layer the crate's layer may not use, as 'crate -> dependency'."""
    may = config["may_depend"]
    layers = config["crates"]
    out = []
    for crate in sorted(graph):
        for dep in sorted(graph[crate]):
            if layers.get(crate) in may and layers.get(dep) in may and layers[dep] not in may[layers[crate]]:
                out.append(f"{crate} -> {dep}")
    return out


def check(config, graph, base=None, optional=frozenset(), upstream=None):
    """(problems, notes): problems fail the check, notes are only printed. `optional` is optional_edges()."""
    may = config["may_depend"]
    layers = config["crates"]
    allowed = set(config.get("allowed", []))
    recorded_optional = optional_rows(config)
    problems = []
    for crate in sorted(graph):
        layer = layers.get(crate)
        if layer is None:
            problems.append(f"{crate}: no layer. Give it one in {CONFIG_PATH} ({DOC}{WHERE} says which)")
        elif layer != TOOL and layer not in may:
            problems.append(f"{crate}: unknown layer {layer!r} in {CONFIG_PATH}")
    for crate in sorted(graph):
        layer = layers.get(crate)
        if layer is None or layer == TOOL or layer not in may:
            continue
        for dep in sorted(graph[crate]):
            # A path dependency with no layer is a vendored crate (third_party/) or a fixture.
            if layers.get(dep) == TOOL:
                problems.append(f"{crate} ({layer}) -> {dep} (tool): shipped code may not depend on a dev tool "
                                f"({DOC}{SECTION[TOOL]})")
    found = set(crossings(config, graph))
    for edge in sorted(found - allowed):
        crate, dep = edge.split(" -> ")
        if edge in recorded_optional:
            if edge not in optional:
                problems.append(f"{edge}: listed in 'optional' in {CONFIG_PATH}, but the dependency is no longer "
                                f"optional. Keep it behind its feature, or move the code to the right layer "
                                f"({DOC}{SECTION[layers[crate]]})")
            continue
        problems.append(f"{edge}: {layers[crate]} may not depend on {layers[dep]}. "
                        f"See {DOC}{SECTION[layers[crate]]}: move the code to the right layer, or offer an "
                        f"extension point the other side plugs into")
    if base is not None:
        base_layers = base.get("crates", {})
        if upstream is not None:
            for crate, layer in sorted(layers.items()):
                if crate not in base_layers and upstream.get("crates", {}).get(crate) != layer:
                    problems.append(f"{crate}: new layer assignment is not listed verbatim in verified upstream")
        # An exception may move between the lists (an `allowed` edge put behind a feature), but not appear.
        base_exceptions = set(base.get("allowed", [])) | optional_rows(base)
        for key, entries in (("allowed", allowed), ("optional", recorded_optional)):
            for edge in sorted(entries - base_exceptions):
                crate, dep = edge.split(" -> ")
                # A crate new since the base (split out of another, or renamed) may carry its old crate's
                # exceptions; a new dependency target excuses nothing.
                restored = (key == "allowed" and upstream is not None
                            and edge in upstream.get("allowed", []))
                if not restored and (crate in base_layers or upstream is not None):
                    problems.append(f"{edge}: added to '{key}' in {CONFIG_PATH}. The exceptions only shrink; "
                                    f"see {DOC}{SECTION.get(layers.get(crate), '')}")
    notes = [f"{edge}: no longer a dependency; you can remove it from 'allowed' in {CONFIG_PATH}"
             for edge in sorted(allowed - found)]
    notes += [f"{edge}: no longer a dependency; you can remove it from 'optional' in {CONFIG_PATH}"
              for edge in sorted(recorded_optional - found - allowed)]
    return problems, notes


def portable(config, packages, base=None):
    """Problems of the portable crates (docs/layers.md#engine): a local dependency on a crate that isn't portable,
     or a dependency on an external library they never take (window, GPU, sound, gamepad, clipboard, HTTP)."""
    spec = config.get("portable", {})
    crates, forbidden = set(spec.get("crates", [])), set(spec.get("forbidden", []))
    optional_baselines = optional_rows(config)
    pending = {crate: set(deps) for crate, deps in spec.get("pending", {}).items() if not crate.startswith("_")}
    problems = {f"{crate}: listed as portable in {CONFIG_PATH}, but not an engine crate there"
                for crate in crates | set(pending) if config["crates"].get(crate) != "engine"}
    problems |= {f"{crate}: in both portable.crates and portable.pending of {CONFIG_PATH}"
                 for crate in crates & set(pending)}
    base_pending = (base or {}).get("portable", {}).get("pending", {})
    base_portable = set((base or {}).get("portable", {}).get("crates", []))
    for crate, deps in pending.items():
        if crate in base_portable and crate not in base_pending and deps:
            problems.add(f"{crate}: portable on the base, now in portable.pending of {CONFIG_PATH} with "
                         f"{', '.join(sorted(deps))}. A portable crate stays portable")
        if crate in base_pending:
            for dep in sorted(deps - set(base_pending[crate])):
                problems.add(f"{crate} -> {dep}: added to portable.pending in {CONFIG_PATH}. "
                             f"The exceptions only shrink")
    for package in packages:
        if package["name"] not in crates and package["name"] not in pending:
            continue
        for dep in package["dependencies"]:
            if dep.get("kind") == "dev":
                continue
            local, name = resolve_local(dep)
            edge = f"{package['name']} -> {name}"
            if dep.get("optional") and edge in optional_baselines:
                continue
            if local and name not in crates | pending.get(package["name"], set()):
                problems.add(f"{edge}: a portable crate depends only on the other portable crates "
                             f"({DOC}{SECTION['engine']})")
            elif not local and name in forbidden:
                problems.add(f"{edge}: a portable crate takes no window, GPU, sound, gamepad, clipboard or HTTP "
                             f"library ({DOC}{SECTION['engine']})")
    return sorted(problems)


def workspace_packages(config):
    # Every crate in every checked workspace, not just each manifest's own members: layers.json's `crates` is the
    # authoritative inventory of local crates, so a patch pointing across workspaces is still a candidate.
    known = set(config["crates"])
    return [package for manifest in config["workspaces"] for package in metadata(ROOT / manifest, known)]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="refuse 'allowed' entries added since this git ref")
    parser.add_argument("--list", action="store_true", help="print each crate's layer and the exceptions")
    if __package__:
        from . import merge_base
    else:
        import merge_base
    parser.add_argument("--pull-request-merge", action="store_true", help=merge_base.PULL_REQUEST_MERGE_HELP)
    args = parser.parse_args(argv)
    if args.base:
        args.base, error = merge_base.pull_request_base(args.base, args.pull_request_merge)
        if error:
            print(f"Layer check failed: {error}")
            return 1
    config = load_config()
    packages = workspace_packages(config)
    graph = edges(packages)
    if args.list:
        for crate in sorted(graph):
            print(f"{config['crates'].get(crate, '?'):<9} {crate}")
        ups = crossings(config, graph)
        print(f"\n{len(ups)} dependencies on a layer the crate's layer may not use:")
        for edge in ups:
            print(f"  {edge}")
        return 0
    base = base_config(args.base) if args.base else None
    optional = optional_edges(packages)
    problems, notes = check(config, graph, base, optional)
    if args.base and (problems or config != base):
        # Only a violation, or an inventory that differs from the base's, can need what a verified sync restored:
        # every other run stays off the network.
        upstream, provenance = merge_base.verified_sync_config(args.base, CONFIG_PATH)
        if upstream is not None or provenance:
            problems, notes = check(config, graph, base, optional, upstream)
        problems = provenance + problems
    problems += portable(config, packages, base)
    for note in notes:
        print(f"note: {note}")
    if problems:
        print(f"Layer check failed ({DOC}):")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"Layer check passed: {len(graph)} crates, {len(config.get('allowed', []))} exceptions left "
          f"to move ({DOC}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
