# Filtering toast notifications

Implement `Hooks::toast_add` to filter a client notification before it enters its
queue (NeoForge `ToastAddEvent`). The hook receives a read-only `ClientHost` and
`ToastAddEvent`: `kind` is a `ToastKind`, and `title` and `description` are mutable
text components. Return `Flow::Cancel` to discard only that candidate, or
`Flow::Continue` after optionally replacing either component. No slot, animation
or toast sound is started for a cancelled candidate.

## Toast families and edits

The families are `Advancement`, `Recipe`, `Tutorial`, `System` and `Other` (including
now-playing music). Match non-exhaustively. For advancement toasts, the title is
the task/goal/challenge label and the description is the advancement's name, not
its advancement-screen description. Recipe notifications keep their rotating
station/result icons; tutorial hints keep their progress bar. Edits change only
the displayed text, not these identities or lifetimes. Setting the optional
description to `None` removes that line.

## Admission rules

The hook runs for new additions, not in-place `addOrUpdate` changes to an existing
recipe or system toast. A cancelled tutorial hint is not retried every frame;
a later, different hint is a new candidate. Title-screen backup/access failures,
narrator notices, pack download/failure and chunk IO notices all use the same
admission point. Candidates produced during the initial resource loading screen wait until
the game can invoke the hook with a `ClientHost`. The pause screen's persistent song label
is HUD content, not a queued toast, and is not filtered by this hook.
