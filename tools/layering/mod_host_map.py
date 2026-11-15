"""The tables docs/layering/mod-host-map.md describes, from scan_engine_mod_edges.py's result (generated, not committed)."""

from collections import Counter, defaultdict
import itertools
from pathlib import Path
import re

STRANDS = ["loading", "host", "slots", "attachments", "content_render", "hooks"]
# How each strand is resolved (arismoko/horizon#2534): phases 2 and 4 are dropped (owner, 2026-11-10); phase 6 is not built.
LABEL = {"loading": "phase 1", "host": "stays in the engine", "slots": "phase 3",
         "attachments": "stays in the engine", "content_render": "phase 5", "hooks": "phase 6, not built"}
HOST_AREAS = ("crates/horizon-client/src/game/modding/", "crates/horizon-client/src/game/native_hooks/")
# What a hook-firing engine file needs of mod_api, from least to most.
DISPATCH = re.compile(r"^mod_api::(HookId|Flow|Hooks|__subscriptions|InteractionResult|\w+(Event|Call|Result)"
                      r"|__private::(listened|has_\w+_hook|built|enter|\w+Call))(::|$)")
TESTING = re.compile(r"^mod_api::(testing|__private::(part_tests|with_test_packages))\b")
MORE = re.compile(r"^mod_api::(\w*Host|HostSide|Backend\w*|RegistrationContext|Registr\w*|ModEntry|Mods|content"
                  r"|__private::(game|install|mods|\w+_key\w*|\w+_id|resolve_\w+|block_definitions|check_access"
                  r"|LoadAccess|ModPhase|PhaseOwner|current_phase|backend\w*|set_mod_enabled|mark_game_thread))\b")


def area(path):
    m = re.match(r"crates/horizon-client/src/game/(\w+)/", path)
    if m:
        return f"game/{m.group(1)}"
    if path == "crates/horizon-client/src/game.rs":
        return "game.rs"
    if path.startswith("crates/horizon-client/"):
        return "client (non-game)"
    return path.split("/")[1]


def table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    out += ["| " + " | ".join(str(c) for c in row) + " |" for row in rows]
    return "\n".join(out)


def tier(path):
    if TESTING.search(path):
        return "testing"
    if DISPATCH.search(path):
        return "dispatch"
    if MORE.search(path):
        return "more"
    return "value"


def render(result):
    edges, inbound, files = result["edges"], result["inbound"], result["move_set"]
    out = ["Counts are source references (one per `use` leaf or path), split into production (prod) and test code."]
    out.append("\n### Manifest edges (engine crate → mod crate)\n")
    out.append(table(["engine crate", "mod crate", "kind", "mod crate's layer", "Cargo.toml line", "strand"],
                     [(e["crate"], e["dep"], e["kind"], e["dep_layer"], e["line"], e["strand"])
                      for e in result["manifest_edges"]]))
    out.append("\n### Source references by engine crate and mod crate\n")
    by = defaultdict(lambda: [set(), 0, 0])
    for e in edges:
        row = by[(e["crate"], e["mod_crate"])]
        row[0].add(e["file"])
        row[2 if e["test"] else 1] += 1
    out.append(table(["engine crate", "mod crate", "files", "prod refs", "test refs"],
                     [(c, m, len(v[0]), v[1], v[2]) for (c, m), v in sorted(by.items())]))
    out.append("\n### By area (horizon-client game areas, other crates whole)\n")
    by = defaultdict(lambda: [set(), 0, 0])
    for e in edges:
        row = by[area(e["file"])]
        row[0].add(e["file"])
        row[2 if e["test"] else 1] += 1
    out.append(table(["area", "files", "prod refs", "test refs"],
                     [(a, len(v[0]), v[1], v[2]) for a, v in sorted(by.items(), key=lambda kv: -kv[1][1] - kv[1][2])]))
    out.append("\n### By strand\n")
    by = Counter((e["strand"], e["test"]) for e in edges)
    host = Counter((e["strand"], e["test"]) for e in edges if e["file"].startswith(HOST_AREAS))
    out.append(table(["strand", "prod refs", "test refs", "of which in modding/ + native_hooks/ (prod)"],
                     [(s, by[(s, False)], by[(s, True)], host[(s, False)]) for s in STRANDS]))
    out.append("\n### Engine files outside modding/ and native_hooks/: what they need of `mod_api`\n")
    out.append("Each file is put in the highest tier it uses in production code: `dispatch` (HookId, Flow, Hooks, "
               "`__subscriptions`, `listened`, event and call structs), `value` (plain API values: handles, "
               "states, stacks, text, keys as types), `more` (Host traits, registration, `ModEntry`, "
               "`__private::game`, key lookups, load phases). `testing` counts test-only items.\n")
    per_file = defaultdict(set)
    test_only = Counter()
    for e in edges:
        if e["file"].startswith(HOST_AREAS) or e["mod_crate"] != "horizon-mod-api":
            continue
        if e["test"]:
            test_only["test code"] += 1
            if TESTING.search(e["path"]):
                test_only["testing items"] += 1
            continue
        per_file[e["file"]].add(e["path"])
    order = ["dispatch", "value", "more"]
    tiers = Counter()
    items = defaultdict(Counter)
    for f, paths in per_file.items():
        ts = {tier(p) for p in paths} - {"testing"}
        top = max(ts, key=order.index) if ts else "testing"
        tiers[top] += 1
        for p in paths:
            items[tier(p)][re.sub(r"^mod_api::", "", p)] += 1
    out.append(table(["highest tier", "files"], [(t, tiers[t]) for t in order + ["testing"] if tiers[t]]))
    for t in order:
        common = ", ".join(f"`{p}` ({n})" for p, n in sorted(items[t].items(), key=lambda kv: (-kv[1], kv[0]))[:25])
        out.append(f"\n- **{t}** items, by files using them: {common}")
    out.append(f"\nOutside the host areas, {test_only['test code']} `mod_api` references are in test code; "
               f"{test_only['testing items']} of them name `testing`, `part_tests` or `with_test_packages`.")
    out.append("\n### The host areas by strand (files under game/modding/ and game/native_hooks/)\n")
    out.append("`inherent impls`: `impl Game`/`impl GameLevel` blocks, which can't leave horizon-client as they "
               "are. `foreign trait impls`: impls of a mod crate's trait, which horizon-mod-host may not write for "
               "an engine type (orphan rule). `to make public`: distinct engine-private functions, `Game` fields "
               "and `game` items the files use (textual estimate). `inbound`: engine references into them.\n")
    rows = []
    for s in STRANDS:
        fs = {p: v for p, v in files.items() if v["strand"] == s}
        public = set(itertools.chain(*[v["outbound"] for v in fs.values()]))
        inb = [e for e in inbound if e["strand"] == s and not e["test"]]
        rows.append((f"{s} ({LABEL[s]})", len(fs), sum(v["lines"] for v in fs.values()),
                     sum(v["inherent_impls"] for v in fs.values()), sum(v["foreign_trait_impls"] for v in fs.values()),
                     len(public), len(inb), len({e["file"] for e in inb})))
    out.append(table(["strand", "files", "lines", "inherent impls", "foreign trait impls", "to make public",
                      "inbound refs (prod)", "inbound files"], rows))
    voc = result["vocabulary"]
    fs = [f for f in voc["files"] if not f["test"]]
    out.append("\n### horizon-mod-api: vocabulary or not (non-test files)\n")
    out.append(table(["", "files", "lines"], [
        ("all", len(fs), sum(f["lines"] for f in fs)),
        ("pure (no generated include, unsafe, process state or non-core crate path)",
         sum(f["pure"] for f in fs), sum(f["lines"] for f in fs if f["pure"])),
        ("includes generator output (OUT_DIR)", sum(bool(f["generated_include"]) for f in fs), ""),
        ("`unsafe`", sum(bool(f["unsafe"]) for f in fs), sum(len(f["unsafe"]) for f in fs)),
        ("process state (static, thread_local, locks, atomics, cells)", sum(bool(f["state"]) for f in fs),
         sum(len(f["state"]) for f in fs)),
    ]))
    out.append("\nDependencies: " + ", ".join(f"`{d['name']}` ({d['kind']}, {d['layer']})" for d in voc["deps"])
               + f"; build script: {'yes' if voc['build_script'] else 'no'}.")
    out.append("\nBlockers by file (generated includes, `unsafe`, state), lines:\n")
    for f in fs:
        parts = [f"{k} {','.join(map(str, f[k][:12]))}{'…' if len(f[k]) > 12 else ''}"
                 for k in ("generated_include", "unsafe", "state") if f[k]]
        if parts:
            out.append(f"- `{Path(f['file']).relative_to('crates/horizon-mod-api')}`: " + "; ".join(parts))
    return "\n".join(out)
