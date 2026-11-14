# Upstream integration checks

## Upstream changes the merge dropped

A conflict resolved to the fork's side drops upstream's change silently: the sync builds, and the fork later looks
like it reverted upstream's work. Every sync runs, on its merge commit, before the senior read:

    python3 tools/sync/dropped_upstream.py <merge commit> --markdown

It lists each upstream commit with a file whose upstream-added lines are missing from the merge (kept fork side,
partly missing, no fork file, an upstream deletion undone, a binary that differs), and every file it could only find
by guessing where the fork moved it. The list over-reports, and it can miss a change that only removes or reorders
lines or whose lines already exist elsewhere in the file: sort each commit by hand
(`git show`, `git grep` at the merge) into carried, rightly kept fork side (say why), or lost, re-apply every lost
one in the sync, and put the table with its "Sorted as" column filled in the sync PR body.

## Sync provenance

`tools/check_layers.py` and `tools/check_addresses.py` (through `tools/merge_base.py`) accept exact rows or
assignments restored from upstream's inventory only for a verified sync. The sync's upstream commit is derived
from the branch's history, with no declaration to write: it is the newest non-first parent of a merge commit in
`merge_base..HEAD` that is not an ancestor of the merge base and is an ancestor of upstream main, freshly fetched
from `https://github.com/horizon-rust/horizon.git` into `refs/sync/upstream-main`. Then `sync_provenance` requires:

- the SHA resolves to an available commit;
- it is an ancestor of upstream main;
- it is not an ancestor of the merge base;
- it is a direct parent of a merge commit in `merge_base..HEAD`.

A branch with no merge of upstream main (a fake SHA, an old base ancestor, a fork-only side-branch merge) has no
sync, and nothing is restored. A PR could still merge a real upstream commit and add unrelated rows: review
confirms that this is an upstream sync and that each restored row is justified.

Both checks look this up lazily: only when they have a violation, or when their inventory file differs from the
merge base's, so an ordinary PR or rollup never touches the network. When they look, an unreachable upstream, a
timeout, a shallow checkout or missing history fails closed, even if a previous fetch left a cached ref.
`HORIZON_UPSTREAM_URL` overrides the URL for local bare-repo tests; normal CI leaves it unset.
