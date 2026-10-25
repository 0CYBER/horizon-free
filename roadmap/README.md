# The roadmap

`roadmap.toml` is the one source of the roadmap. `tools/roadmap.py build` turns it into
[ROADMAP.md](../ROADMAP.md); the website's hub (playhorizon.dev/hub) and Kira read the same items. The work
board stays the task-level view (every issue and pull request); the roadmap is the overview, and each pull
request carries the `roadmap:<id>` label of the item it moves.

## An item

```toml
[[item]]
id = "server-dialogs"            # lowercase words joined by dashes; never renamed (votes and links use it)
title = "Server dialogs and login screens"
status = "now"                   # now | next | later | help-wanted | shipped
area = "servers"                 # one of the [[area]] ids, listed in the order of priority
focus = true                     # optional: one of the 3 to 5 Now items in the overview
ongoing = true                   # optional: a workstream that goes on after its pull requests merge
plain = """Servers that show dialogs ... will work in Horizon."""   # players' words, at most 240 characters
dev = """26.3's dialog packets and model (#1720), ..."""             # crates, approach, what is left
links = ["#1720", "#1734", "docs/hosting.md"]                       # #N, a repository path, or https://
shipped_in = "v0.2.58"           # shipped items only: a game tag, launcher@x.y.z, or "website"
post = "..."                     # optional: the announcement when it ships, if `plain` doesn't read as one
```

- **No dates, ever**: Now, Next, Later. The check rejects month names, years, quarters and "next week".
  "Coming soon" is the only phrase for something waiting on someone else (Microsoft sign-in, CurseForge).
- **Only what is decided.** Ideas stay in issues and in the suggestion queue until they are.
- **Only what is true in a release** for shipped items: a merged pull request isn't shipped until a release has it.
- `plain` is for players: what they will be able to do, not how. `dev` is for contributors: the code, the
  vanilla classes, the pull requests, what is left. Both stay public: no machines, accounts or private notes.
- Help wanted items are good first contributions: a clear vanilla behaviour, an issue with steps, a place to
  start in `dev`.

## Keeping it current

Whoever moves the work (the orchestrator on each heartbeat, or anyone in their pull request):

1. `python3 tools/roadmap.py sync`: every item whose linked pull requests all went out in a release, whose
   linked issues are all closed and that isn't `ongoing` becomes `shipped`, with that release in
   `shipped_in`, on top of the shipped items. It rewrites `roadmap.toml` and ROADMAP.md; open a pull request
   with the change (`skip-release`).
2. `python3 tools/roadmap.py check`: the file's rules, then GitHub's state of every link. It fails when a
   link is not an issue or pull request, when a Now item has no open pull request and none merged in the last
   14 days (link the new work, or move it to Next or Later), and when every link of a Help wanted item is
   closed. It also says which items `sync` will ship once a release has them.
3. When work starts, opens a pull request or is decided: add or move the item, add its links, then
   `python3 tools/roadmap.py build`. Keep the overview's focus to the 3 to 5 biggest Now items.
4. `python3 tools/roadmap.py labels` (after the change merges): creates a `roadmap:<id>` label for each new
   item, updates their colour (by status) and description, and adds the label to the open pull requests an item
   links. Label a new pull request for an item by hand or by linking it and running `labels` again. Labels of
   removed items are listed, never deleted.
5. Suggestions from the website and Discord are read and triaged by the maintainers. A good one becomes an
   item in a normal pull request; none is published automatically.

CI runs `tests/test_roadmap.py` (in the Python tests), which fails when ROADMAP.md is not what `build` writes
or when the file breaks a rule; `python3 tools/roadmap.py check --offline` runs the same rules without GitHub.
