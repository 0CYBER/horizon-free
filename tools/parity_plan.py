#!/usr/bin/env python3
"""Split a parity selection over several runners, and export the dependency lists a fresh runner's run traced, for
runners without parity state of their own to select what a change reaches.

  python3 tools/parity_plan.py --plan plan.json -- --changed origin/main   # the shards this selection needs
  python3 tools/parity_plan.py --shard 2/5 -- --all                        # the names in shard 2 of 5, one per line
  python3 tools/parity_plan.py --export-dependencies DIR                   # a fresh runner's dependency lists

The selection is parity.py's own: the arguments after `--` go to `parity.py --list`, so nothing is built or run.
What parity.py leaves out of it (named only, skipped on this platform) is listed with its reason, as not run.
Each harness joins the group of the probe it builds (game, client, world, other; from its record here or the shared
dependency list in PARITY_SHARED), and a shard holds one group, so a runner pays only that group's cold build.
Runners then run `parity.py <names>`. This file is never imported by parity.py or a harness: a change to it reruns
no harness.
"""
import argparse
import json
import math
import re
import shutil
import subprocess
import sys
from pathlib import Path

try:
    from . import parity
except ImportError:
    import parity

# Harnesses per shard for about 20 minutes on a cold 16-vCPU hosted runner with no GPU (measured 2026-11-19 on
# blacksmith-16vcpu-ubuntu-2404, 8 harnesses at a time): ~60 s of setup and data, then the group's cold build (world
# ~40 s; client ~530 s for client_probe's fast and dev profiles; game ~480 s for its two builds), then 3-10 s of
# runner time per harness, nearly all of it in the vanilla oracle. other: probes nothing recorded names.
GROUPS = ("game", "client", "world", "other")
SHARD_SIZES = {"game": 50, "client": 100, "world": 150, "other": 50}
GAME_PACKAGES = {"horizon-game", "horizon-client", "horizon-client-lib"}
# GitHub starts at most 256 jobs from one matrix.
MATRIX_LIMIT = 256
# The lines parity.py --list prints for what it leaves out (a test pins them to parity.py's own wording).
NAMED_ONLY_LINE = re.compile(r"not selected \(runs when named, and nightly\): ([^\s:]+): (.*)")
SKIPPED_LINE = re.compile(r"(skipped on .+?): ([^\s:]+): (.*)")


def selection(arguments):
    """(the harnesses `parity.py ARGUMENTS --list` would run, in its order; {name: reason} for those it left out).
    Exits as parity.py does on a bad argument."""
    result = subprocess.run([sys.executable, str(Path(parity.__file__)), *arguments, "--list"], cwd=parity.ROOT,
                            capture_output=True, text=True)
    sys.stderr.write(result.stderr)
    if result.returncode:
        sys.exit(result.returncode)
    found = parity.harnesses()
    names, left_out = [], {}
    for line in result.stdout.splitlines():
        if line in found:
            names.append(line)
        elif match := NAMED_ONLY_LINE.fullmatch(line):
            left_out[match[1]] = f"named only: {match[2]}"
        elif match := SKIPPED_LINE.fullmatch(line):
            left_out[match[2]] = f"{match[1]}: {match[3]}"
    return names, left_out


def artifacts(name, records):
    """The probes a harness built: its record here, else the shared dependency list (as parity.py prebuilds them)."""
    record = records.get(name)
    if not record or "artifacts" not in record:
        template = parity.SHARED / "dependencies" / f"{name}.json"
        record = json.loads(template.read_text(encoding="utf-8")) if template.exists() else {}
    return parity.recorded_artifacts(record)


def probe_group(built):
    """game when it builds the game's executables, else client or world for that crate's probe, else other."""
    def packages(artifact):
        chosen = artifact[3] if len(artifact) > 3 else []
        return {value for flag, value in zip(chosen, chosen[1:]) if flag == "-p"}
    # A binary built with no -p may be any workspace crate's: counted as the game.
    if any(artifact[0] in ("bin", "bins") and (packages(artifact) & GAME_PACKAGES or not packages(artifact))
           for artifact in built):
        return "game"
    names = {artifact[1] for artifact in built}
    every = set().union(*map(packages, built)) if built else set()
    if "client_probe" in names or every & {"horizon-client", "horizon-client-lib"}:
        return "client"
    if "world_probe" in names or "horizon-world" in every:
        return "world"
    return "other"


def shards_needed(names, groups):
    return sum(math.ceil(sum(groups[name] == group for name in names) / SHARD_SIZES[group]) for group in GROUPS)


def plan(names, groups, count):
    """count shards of names: [(group, [names])], each group's shards together, in GROUPS order.

    Every group with harnesses gets a shard of its own while there are enough; the rest go one at a time to the group
    whose shards are fullest for their size. A group's harnesses are dealt in turn, so the entries of a sharded
    harness (world_spawn#1...) spread over its shards. With fewer shards than groups, whole groups share shards,
    the largest first into the emptiest."""
    if count < 1:
        raise ValueError("a plan needs at least one shard")
    members = {group: [name for name in names if groups[name] == group] for group in GROUPS}
    present = [group for group in GROUPS if members[group]]
    if not present:
        return [("", [])] * count
    if count < len(present):
        shards = [([], []) for _ in range(count)]
        for group in sorted(present, key=lambda group: (-len(members[group]), GROUPS.index(group))):
            labels, harnesses = min(shards, key=lambda shard: len(shard[1]))
            labels.append(group)
            harnesses += members[group]
        return [("+".join(labels), [name for name in names if name in set(harnesses)]) for labels, harnesses in shards]
    counts = {group: 1 for group in present}
    for _ in range(count - len(present)):
        fullest = max(present, key=lambda group: (len(members[group]) / (counts[group] * SHARD_SIZES[group]),
                                                  -GROUPS.index(group)))
        counts[fullest] += 1
    return [(group, members[group][index::counts[group]]) for group in present for index in range(counts[group])]


def hosted_left_out(names, config):
    """{name: reason} for the selected harnesses hosted runners don't run: those the "hosted" entry of
    tools/parity_hosts.json excludes (data their runners lack), left out as playhorizon's excludes are there, and
    the known failing and named-only ones."""
    reasons = {**{name: f"named only: {why}" for name, why in parity.named_only(config).items()},
               **{name: f"known failing: {why}" for name, why in parity.known_failing(config).items()},
               **parity.platform_skips(config, "hosted")}
    return {name: reasons.get(name) or reasons[parity.base_name(name)] for name in names
            if name in reasons or parity.base_name(name) in reasons}


class IncompleteExport(Exception):
    pass


def export_dependencies(records, found, directory):
    """Write NAME.json ({"dependencies", "artifacts"}, as parity.py's shared lists) for each harness passed here.

    Only meaningful on a runner that started without parity state and ran parity.py once: every record is then that
    run's, traced at this commit. Complete or nothing: it raises IncompleteExport, writing nothing, when a record was
    taken from the shared folder (state this run did not trace), a pass has no dependency or probe list (its trace
    was lost), or DIR is not empty. A failed harness gets no list (its trace may stop early): a runner without its
    list counts it as reached by every change. If a write fails, DIR is removed. Returns how many lists."""
    problems = [f"{name}: its pass was taken from the shared folder, not traced by this run"
                for name in sorted(found) if (records.get(name) or {}).get("shared")]
    passed = [name for name in sorted(found) if (records.get(name) or {}).get("status") == "pass"
              and not records[name].get("shared")]
    problems += [f"{name}: passed without a dependency or probe list" for name in passed
                 if not records[name].get("dependencies") or "artifacts" not in records[name]]
    if directory.exists() and any(directory.iterdir()):
        problems.append(f"{directory} is not empty")
    if problems:
        raise IncompleteExport("; ".join(problems))
    directory.mkdir(parents=True, exist_ok=True)
    try:
        for name in passed:
            data = {"dependencies": records[name]["dependencies"], "artifacts": records[name]["artifacts"]}
            (directory / f"{name}.json").write_text(json.dumps(data), encoding="utf-8")
    except OSError as error:
        shutil.rmtree(directory, ignore_errors=True)
        raise IncompleteExport(f"writing the lists failed ({error}); {directory} removed") from error
    return len(passed)


def shard_spec(text):
    match = re.fullmatch(r"(\d+)/(\d+)", text)
    if not match or not 1 <= int(match[1]) <= int(match[2]):
        raise argparse.ArgumentTypeError(f"--shard takes I/N with 1 <= I <= N, not {text!r}")
    return int(match[1]), int(match[2])


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    ours, rest = (argv[:argv.index("--")], argv[argv.index("--") + 1:]) if "--" in argv else (argv, [])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--plan", metavar="FILE", help="write the shards the selection needs as JSON, and list them")
    action.add_argument("--shard", type=shard_spec, metavar="I/N", help="print the names in shard I of N")
    action.add_argument("--export-dependencies", metavar="DIR", type=Path,
                        help="write the dependency lists of the harnesses passed here into DIR (empty or new; no "
                             "selection). Only meaningful on a runner that started without parity state and ran "
                             "parity.py once; exits 1, writing nothing, when the export would be incomplete")
    parser.add_argument("--hosted", action="store_true",
                        help="leave out what hosted runners don't run (hosted_left_out), and list it")
    args = parser.parse_args(ours)
    records = parity.load_state()["harnesses"]
    if args.export_dependencies:
        try:
            count = export_dependencies(records, parity.harnesses(), args.export_dependencies)
        except IncompleteExport as error:
            print(f"no dependency lists exported: {error}", file=sys.stderr)
            return 1
        print(f"{count} dependency lists written to {args.export_dependencies}")
        return 0
    names, left_out = selection(rest)
    if args.hosted:
        left_out.update(hosted_left_out(names, parity.host_config()))
    for name, reason in left_out.items():
        print(f"not run here: {name}: {reason}", file=sys.stderr if args.shard else sys.stdout)
    names = [name for name in names if name not in left_out]
    groups = {name: probe_group(artifacts(name, records)) for name in names}
    if args.shard:
        index, count = args.shard
        for name in plan(names, groups, count)[index - 1][1]:
            print(name)
        return 0
    count = shards_needed(names, groups)
    if count > MATRIX_LIMIT:
        print(f"{len(names)} harnesses need {count} shards, more than the {MATRIX_LIMIT} jobs GitHub starts from one "
              "matrix: name fewer harnesses", file=sys.stderr)
        return 1
    shards = plan(names, groups, count) if names else []
    Path(args.plan).write_text(json.dumps({"shards": [{"group": group, "harnesses": harnesses}
                                                      for group, harnesses in shards], "left_out": left_out},
                                          indent=1) + "\n", encoding="utf-8")
    for index, (group, harnesses) in enumerate(shards, 1):
        print(f"shard {index}/{len(shards)} {group}: {len(harnesses)}: {' '.join(harnesses)}")
    print(f"{len(names)} harnesses in {len(shards)} shards: "
          + ", ".join(f"{sum(groups[name] == group for name in names)} {group}" for group in GROUPS))
    return 0


if __name__ == "__main__":
    sys.exit(main())
