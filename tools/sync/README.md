# Upstream integration checks

`audit_launcher_restoration.py --upstream <sha> --upstream-base <sha>` inventories
changed launcher files and fork commits. It marks whole upstream takes without
judging whether a fork hunk is a permitted Horizon-layer restoration or mod work
that must remain. Reviewers check the inventory and the resulting corrections.

## Upstream changes the merge dropped

A conflict resolved to the fork's side drops upstream's change silently: the sync builds, and the fork later looks
like it reverted upstream's work. Every sync runs, on its merge commit, before the senior read:

    python3 tools/sync/dropped_upstream.py <merge commit> --markdown

It lists each upstream commit with a file whose upstream-added lines are missing from the merge (kept fork side,
partly missing, no fork file, or an upstream deletion undone). The list over-reports: sort each commit by hand
(`git show`, `git grep` at the merge) into carried, rightly kept fork side (say why), or lost, re-apply every lost
one in the sync, and put the table with its "Sorted as" column filled in the sync PR body.

## File-size baseline provenance

`tools/check_file_size.py --base <base>` permits additions or raised caps only
when the full SHA in `tools/file_size_baseline.txt` changes and:

- the SHA resolves to an available commit;
- it is an ancestor of upstream main, freshly fetched from
  `https://github.com/horizon-rust/horizon.git` into `refs/sync/upstream-main`;
- it is not an ancestor of the merge base;
- it is a direct parent of a merge commit in `base..HEAD`.

A fake SHA, an old base ancestor, an ordinary new ancestor without a merge,
and missing provenance all fail. In a sync PR, entries above the current non-test size fail
as stale-high, including entries for deleted files. Outside a sync, entries are ignored;
adding or raising one still fails, and shrinking a listed file needs no baseline edit.
The PR body lists every sync cap and its growth for human review.

## Regenerating the baseline

Only a sync PR uses `tools/file_size_baseline.txt`: when its header is unchanged against the merge
base, the checker ignores the entries and holds every file to its size on the base, so a PR that shrinks a
listed file does not touch the file. A sync declares its upstream commit in the header and the entries cap
the files that grew past the limit. Do not edit entries by hand. Regenerate:

    python3 tools/sync/write_file_size_baseline.py --base origin/main --marker <full upstream SHA>

The command writes one entry, at the exact current size, for each Rust file over 3000 code lines that grew past
its size on the merge base, skipping generated files and `third_party/`. `--marker` is required when something
grew and must be new compared with the base; without growth the header is kept and the file has no entries.
`--check` prints the result without writing it.

On a conflict in the baseline (a rollup that carries a sync, or a sync merged with a PR that touched it): take
either side, run the command on the merged tree, and commit the result as its own commit. Run it again at the
end of a sync, after the last code change, and run `python3 tools/check_file_size.py --base origin/main`.

A fork-only side-branch merge cannot establish upstream provenance. A PR could
still merge a real upstream commit and add unrelated caps: review must confirm
that this is an upstream sync and that the listed restoration growth is justified.

The fetch is required on every changed sync declaration; an unreachable upstream,
a timeout or missing history fails closed, even if a previous fetch left a cached
ref. Dyna verify therefore fails, with a network-required message, if its runner
cannot reach GitHub. `HORIZON_UPSTREAM_URL` overrides the URL for local bare-repo
tests; normal CI leaves it unset.

The Dyna verify checkout initially uses the checkout action's shallow default,
then its `Fetch this commit's history` step fetches `HEAD` with `--unshallow` before
running the file-size check. This retains the upstream-parent merge in history.
The checker refuses a still-shallow checkout with an explicit full-history error;
it never treats missing history as approval.
