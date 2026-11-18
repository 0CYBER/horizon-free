#!/usr/bin/env python3
"""Reject local game artifacts even when somebody uses git add -f."""

import argparse
import collections
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile

PRIVATE_ROOTS = {".local", "decompiled", "minecraft", "assets", "run", "logs"}
PRIVATE_EXTENSIONS = {".jar", ".class", ".jfr", ".hprof", ".java"}
ORIGINAL_JDK_TOOLS = {
    # Original observer of public Paper API events, with no server implementation code.
    "tools/fixtures/paper-trace-plugin/PaperTrace.java",
    "tools/JfrSummary.java",
    # Original generator of the Java character tables: calls only the public Character API and prints ranges; no JDK implementation source.
    "tools/JavaCharacterTables.java",
    "tests/java/JfrFixture.java",
    # HISTORY-ONLY: this original comparison driver is no longer in the current tree.
    # Keep its old path allowed so --history/--pre-push accept its legitimate historical addition.
    "tools/refactors/api-pr3/JsonNbtOracle.java",
}
# The prepared folder's files in one file (crates/horizon-core/src/prepared_files.rs): Mojang's data.
PRIVATE_NAMES = {"horizon-data.bin"}
# Vanilla's 1.13 data fixer tables, copied from its code until 2026-11; horizon-prepare now exports them from the
# player's jar (datafix-tables.json). Their old paths stay in history, so only the current tree is checked.
RETIRED_TABLES = {"crates/horizon-world/src/datafix/flat_tables.rs",
                  "crates/horizon-world/src/datafix/block_state_data.txt"}
# A file with this many entries like those tables' holds a copy of them, whatever its name or format:
# ItemStackTheFlatteningFix's `id.damage` keys, and BlockStateData's `id<TAB>name<TAB>` rows (raw or JSON-escaped).
TABLE_FINGERPRINTS = [r"minecraft:[a-z_]+\.[0-9]+", "[0-9]+(\t|\\\\t)minecraft:[a-z_]+(\t|\\\\t)"]
TABLE_ENTRIES = 50
FINGERPRINT_RES = [re.compile(pattern) for pattern in TABLE_FINGERPRINTS]


def prohibited(path):
    parsed = PurePosixPath(path)
    # Original JDK-only tools are an exact allowlist; recovered game Java stays private.
    if path in ORIGINAL_JDK_TOOLS:
        return False
    # Original Horizon mod sources may also be tracked.
    if parsed.suffix == ".java" and path.startswith(("mods/horizon/src/", "mods/horizon-lab/src/")):
        return not ("/dev/horizonlab/" in path)
    return (
        bool(parsed.parts and parsed.parts[0] in PRIVATE_ROOTS)
        or parsed.suffix.lower() in PRIVATE_EXTENSIONS
        or parsed.name in PRIVATE_NAMES
    )


def copied_tables():
    """Tracked files (in the index) that hold vanilla's retired data fixer tables."""
    found = {path for path in paths_in_repository() if path in RETIRED_TABLES}
    for pattern in TABLE_FINGERPRINTS:
        # Every match, not matching lines: a minified JSON file is one line.
        result = subprocess.run(["git", "grep", "--cached", "-I", "-z", "-o", "-E", "-e", pattern],
                                capture_output=True, text=True)
        if result.returncode not in (0, 1):
            raise RuntimeError(f"git grep failed: {result.stderr.strip()}")
        counts = collections.Counter(line.split("\0", 1)[0] for line in result.stdout.splitlines())
        found.update(path for path, count in counts.items() if count >= TABLE_ENTRIES)
    return found


def git_output(*args):
    """Run a read-only git command, failing closed when the object database cannot be read."""
    result = subprocess.run(["git", "--no-replace-objects", *args], capture_output=True)
    if result.returncode != 0:
        raise RuntimeError(f"git {args[0]} failed: {result.stderr.decode('utf-8', 'replace').strip()}")
    return result.stdout


def tree_blobs(tree):
    """Blob ids in a tree, NUL-delimited so odd paths cannot split a record."""
    out = git_output("ls-tree", "-r", "-z", tree)
    found = set()
    for entry in out.decode("utf-8", "surrogateescape").split("\0"):
        if "\t" not in entry:
            continue
        metadata, _path = entry.split("\t", 1)
        parts = metadata.split()
        if len(parts) >= 3 and parts[1] == "blob":
            found.add(parts[2])
    return found


def baseline_blobs(revisions, range_spec):
    """Blobs the publisher already had before the range: never call these newly introduced."""
    if range_spec:
        base = range_spec.split("..", 1)[0]
        trees = [git_output("rev-parse", "--verify", f"{base}^{{tree}}").decode().strip()]
    else:
        roots = git_output("rev-list", "--max-parents=0", *revisions).decode().split()
        trees = [git_output("rev-parse", "--verify", f"{root}^{{tree}}").decode().strip() for root in roots]
    baseline = set()
    for tree in trees:
        baseline.update(tree_blobs(tree))
    return baseline


def candidate_blobs(revisions, baseline):
    """(path, blob id) of added/modified blobs in the published revisions, excluding grandfathered ones.

    --raw -z gives NUL-delimited records, --no-renames makes a rename an add plus a delete so both
    paths are seen, and -m includes merge-resolution-only changes. Only the changed blobs are read,
    never a full tree per commit.
    """
    # Raw merge-parent diffs enumerate the actual outgoing blobs without constructing patches.
    # A -G shortlist without merge diffs misses merge-only additions; adding -m makes it spend
    # minutes diffing every large historical merge. Scan each unique changed blob instead, with
    # table_blobs()' cheap literal filters before the fingerprint regexes. This keeps every merge
    # parent and the full published history in scope, including tables later deleted.
    # Full object ids: baseline blobs come from ls-tree (40 hex in SHA-1, 64 in SHA-256 repos), so a
    # truncated candidate id would never compare equal and a grandfathered blob would look new.
    out = git_output("log", "--format=", "--raw", "-z", "-m", "--no-renames", "--no-abbrev",
                     *revisions, "--")
    fields = out.decode("utf-8", "surrogateescape").split("\0")
    candidates = set()
    for index in range(0, len(fields) - 1, 2):
        metadata, path = fields[index].lstrip("\n"), fields[index + 1]
        if not metadata.startswith(":") or not path:
            continue
        parts = metadata[1:].split()
        if len(parts) < 5 or parts[4] not in ("A", "M", "T"):
            continue
        if parts[1] == "160000":  # A gitlink names a commit, not a file blob.
            continue
        blob = parts[3]
        # Retired paths are the documented legacy exception: allowed in history, denied in the tree.
        if path in RETIRED_TABLES or blob in baseline:
            continue
        candidates.add((path, blob))
    return sorted(candidates)


def table_blobs(shas):
    """Blob ids among `shas` that hold a retired data fixer table.

    One streaming batch process: read a blob, count its fingerprint matches, discard it. Nothing
    scales with the size of a whole tree, and no content is retained after it has been counted.
    """
    if not shas:
        return set()
    # File-backed stdin lets git run at full speed: no per-blob write/flush round trip. File-backed
    # stderr cannot fill up and block the child while we read stdout, so shutdown cannot deadlock.
    with tempfile.TemporaryDirectory() as directory:
        ids = Path(directory) / "ids"
        ids.write_text("\n".join(shas) + "\n")
        with ids.open("rb") as stream, tempfile.TemporaryFile() as errors:
            process = subprocess.Popen(["git", "--no-replace-objects", "cat-file", "--batch"],
                                       stdin=stream, stdout=subprocess.PIPE, stderr=errors)
            found = set()
            try:
                for sha in shas:
                    header = process.stdout.readline().decode("utf-8", "replace")
                    if not header:
                        raise RuntimeError(f"git cat-file: no response for {sha}")
                    if header.endswith(" missing\n"):
                        raise RuntimeError(f"git cat-file: missing blob {sha}")
                    parts = header.split()
                    if len(parts) != 3 or parts[1] != "blob":
                        raise RuntimeError(f"git cat-file: unexpected header {header!r}")
                    size = int(parts[2])
                    content = process.stdout.read(size)
                    if process.stdout.read(1) != b"\n":  # record terminator
                        raise RuntimeError(f"git cat-file: unterminated record for {sha}")
                    if len(content) != size:
                        raise RuntimeError(f"git cat-file: short read for {sha}")
                    if b"\0" in content:  # git grep -I skips binary files too
                        continue
                    # Both fingerprints require this literal; the C-level search skips most blobs
                    # before any decoding or regex work, without changing which blobs can match.
                    if b"minecraft:" not in content:
                        continue
                    text = content.decode("utf-8", "replace")
                    # Every match contains this literal; the C-level count skips most files before
                    # the (much costlier) regex scan. Early exit keeps a table's worth decisive.
                    if text.count("minecraft:") < TABLE_ENTRIES:
                        continue
                    # copied_tables() requires TABLE_ENTRIES matches for a single fingerprint (OR),
                    # not a total split across both, so 25 + 25 is not a copied table.
                    for regex in FINGERPRINT_RES:
                        count = 0
                        for _ in regex.finditer(text):
                            count += 1
                            if count >= TABLE_ENTRIES:
                                found.add(sha)
                                break
                        if sha in found:
                            break
            finally:
                # Closing our own read end makes cat-file exit on its next write; never signal any
                # process we do not own. File-backed stderr needs no draining.
                process.stdout.close()
                process.wait()
            if process.returncode != 0:
                errors.seek(0)
                detail = errors.read().decode("utf-8", "replace").strip()
                raise RuntimeError(f"git cat-file exited {process.returncode}: {detail}")
            return found


def table_blobs_in_revisions(revisions, range_spec):
    """Paths whose newly introduced blobs hold retired data fixer tables, even if later deleted."""
    baseline = baseline_blobs(revisions, range_spec)
    candidates = candidate_blobs(revisions, baseline)
    guilty = table_blobs(sorted({blob for _, blob in candidates}))
    return {path for path, blob in candidates if blob in guilty}


def paths_in_repository(history=False, revisions=None):
    if not history:
        result = subprocess.check_output(["git", "ls-files", "-z"])
        return result.decode("utf-8").split("\0")
    # An empty push (including deletions only) must not fall back to HEAD or --all.
    if revisions is not None and not revisions:
        return []
    roots = ["--all"] if revisions is None else revisions
    # Include diffs against merge parents so merge-resolution-only artifacts are caught.
    # --no-renames: a rename is listed as a deletion and an addition, so both paths still show, and git skips its
    # rename search on every large merge diff (53 s -> 8 s on main's whole history).
    # Git pushes the stored objects, not the history presented by local replace refs.
    result = subprocess.check_output(["git", "--no-replace-objects", "log", "--format=", "--name-only",
                                      "-z", "-m", "--no-renames", *roots, "--"])
    return [path.lstrip("\n") for path in result.decode("utf-8").split("\0")]


def pushed_revisions(stream):
    """Read Git's pre-push ref updates and retain the non-deleted local object IDs."""
    revisions = []
    for line in stream:
        _local_ref, local_oid, _remote_ref, _remote_oid = line.split()
        if local_oid.strip("0"):
            revisions.append(local_oid)
    return revisions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--history", action="store_true", help="Inspect paths in the history reachable from HEAD")
    mode.add_argument("--pre-push", action="store_true", help="Inspect history reachable from Git's ref updates on stdin")
    # CI's pull requests: only the commits a merge would add, not every ref's whole history.
    mode.add_argument("--range", metavar="BASE..HEAD", help="Inspect the commits of a revision range")
    args = parser.parse_args()
    # CI checks out every branch, so `--all` would fail main on any other branch's file: scan HEAD's ancestry.
    revisions = (pushed_revisions(sys.stdin) if args.pre_push else [args.range] if args.range
                 else ["HEAD"] if args.history else None)
    history = args.history or args.pre_push or bool(args.range)
    # Millions of entries in a long history repeat a few thousand paths: judge each path once.
    rejected = sorted(path for path in set(paths_in_repository(history, revisions)) if path and prohibited(path))
    tables = set()
    if history:
        # History/range/push modes judge only the selected committed objects. The index can hold
        # staged work that is not published, so inspecting it here would reject a deletion-only or
        # empty push for unrelated staged content. No revisions means no table scan at all.
        if revisions:
            tables.update(table_blobs_in_revisions(revisions, args.range))
    else:
        tables.update(copied_tables())
    if tables:
        print("Vanilla's data fixer tables come from the player's jar (horizon-prepare), not from Git:",
              file=sys.stderr)
        for path in tables:
            print(f"  {path}", file=sys.stderr)
        return 1
    if rejected:
        print("Private game/profiling artifacts must stay outside Git:", file=sys.stderr)
        for path in rejected:
            print(f"  {path}", file=sys.stderr)
        return 1
    print("Public tree check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
