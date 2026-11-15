# Upstream integration checks

`audit_launcher_restoration.py --upstream <sha> --upstream-base <sha>` inventories
changed launcher files and fork commits. It marks whole upstream takes without
judging whether a fork hunk is a permitted Horizon-layer restoration or mod work
that must remain. Reviewers check the inventory and the resulting corrections.

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

The fetch is required whenever the branch merges a commit the merge base lacks; an unreachable upstream, a
timeout, a shallow checkout or missing history fails closed, even if a previous fetch left a cached ref.
`HORIZON_UPSTREAM_URL` overrides the URL for local bare-repo tests; normal CI leaves it unset.

The Dyna verify checkout initially uses the checkout action's shallow default, then its
`Fetch this commit's history` step fetches `HEAD` with `--unshallow` before running the checks, which retains
the upstream-parent merge in history.
