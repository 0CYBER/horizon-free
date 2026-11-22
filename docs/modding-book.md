# Writing the modding book

The modding book (`docs/modding`, read as Markdown in this repository; `modding-book.yml` checks that it builds) is the manual for mod
authors: it describes what the game does on main, nothing else. Designs, proposals, plans, trackers and coverage
ledgers live in [`docs/design/`](design/) (mod API designs in [`docs/design/mod-api/`](design/mod-api/)), and
notes for people working on Horizon itself in [`docs/mod-api/`](mod-api/). The book build refuses a page whose
title reads as a design, a plan or a work list.

## Where each doc lives

The book follows [Diátaxis](https://diataxis.fr): a page is one of four kinds, and does only that job.

| Kind | Job | Lives in |
|---|---|---|
| Tutorial | Take a newcomer to a working result | Getting started (`start/`) |
| Explanation | Say how the pieces fit, with no steps | Concepts (`concepts/`) |
| How-to | One task, steps only | How-to (`how-to/`), grouped by what the mod changes |
| Reference | Facts, complete and dry | Reference (`reference/`), and the rustdoc of `horizon-mod-api` |

- **A hook, event or `Host` method:** its `///` lines in the declaration under `crates/horizon-mod-vocab/src/hooks/`.
  The generator writes it as `///` on the generated item; `cargo doc -p horizon-mod-api` shows it. A hook without
  a `doc` fails the build.
- **A hook whose game target has many parameters:** a `native_*` target takes at most 7 parameters (receiver included;
  `MAX_TARGET_PARAMETERS` in `crates/horizon-mod-api-gen/src/dispatch.rs`). A hook with more declares its parameters
  on the hook (`fn update_shape(dimension: LevelDimension, ...);`, as written in `hooks/*.rs`) and the target takes them
  as one generated `<Name>Call` (`native_update_shape(&self, ticks, call: UpdateShapeCall<'_>)`),
  handed to `dispatch_<name>(host, call)`. The generator refuses a flat target over the limit, and a target never carries
  `#[allow(clippy::too_many_arguments)]`. The hooks that were already over the limit are listed in
  `FLAT_TARGETS_OVER_THE_LIMIT` in the same file; that list only shrinks (convert a hook, delete its line) and a new hook
  is never added to it. The carrier is host-internal plain data, never on wasm, and no modder sees it.
- **Hook notes:** keep longer prose in a `#[notes = r#"..."#]` attribute on its hook or helper declaration,
  heading included. General catalog prose lives in `hooks/notes.rs`. `mdbook build docs/modding` renders
  the Hook notes chapter from those attributes without writing a Markdown source page. To preview just
  that chapter, run `cargo run --quiet -p xtask -- mod-api notes`; do not commit its output.
- **A guide page:** `docs/modding/src/`. Code in a page is `{{#include}}`d from `docs/modding/examples`, which
  compiles, so a change to the API that breaks an example fails the build.
- **A mod API change** updates its fragment `doc` and any page it affects in the same PR.
- **A new page:** add the file to the folder of its topic. A page `X.md` with a folder `X/` beside it gets the
  folder's pages as its sub-chapters, in file name order (a numeric prefix such as `07-` sets the order where it
  matters), titled by each page's `#` heading; `<!-- section-pages -->` in `X.md` becomes a list of them. A page
  directly in `concepts/` or `how-to/` that `SUMMARY.md` doesn't list joins the end of its part, by title. Any
  other new page needs its `SUMMARY.md` line: the build fails on a page the book doesn't render, and on a
  `SUMMARY.md` link to a missing file.
- **Links:** relative between book pages (inline or reference-style; root-relative links are refused), and
  a github.com URL for anything else in the repository, since the site publishes only `docs/modding/src`. The
  build checks every link and anchor.
- **Images:** SVG under `docs/modding/src/images/`, with alt text that says what the picture says. Text sits on its
  own filled box so a figure reads on the light and the dark themes alike.
- `src/llms.txt` is the index for agents; keep it in step with `SUMMARY.md`.

Build the book with `mdbook build docs/modding` and the reference with `cargo doc -p horizon-mod-api --no-deps`.

## How the book reads

The book is the manual a mod author reads: it says what the game does, and how to use it.

- **Present tense, second person.** "The hook fires after…", "You declare…". Imperative for steps.
- **No status words** (provisional, not built yet, planned, for now, today, currently, as of, still, no longer,
  first slice, follow-up). A feature that does not exist is not mentioned. A request the game refuses is
  documented as behaviour: what is refused, when, and the message or error the author sees.
- **No history:** no "used to", "replaces the old…", migration notes or comparisons with an earlier Horizon.
  Naming the NeoForge or vanilla counterpart of a concept is welcome.
- **No issue or PR numbers, dates, commit hashes, rulings, review notes, test names or benchmark logs,** and no
  instructions for people changing Horizon itself: those go in `docs/mod-api/`.
- **Short.** Lead with what the reader can do and one example; then rules and limits as a list or table.
- **Names exactly as in code,** in backticks. When the code and a page disagree, the code wins: fix the page.
- **Code samples** are `{{#include}}`d from `docs/modding/examples`, which compiles. An inline sample that is not
  compiled is fenced `rust,ignore` and still matches the real signatures.

## What a PR needs to show

Code review, CI and your own focused tests are the proof. You don't need to run the game.

A reviewer runs your change in game at your head only when a test can't cover one of these, and says which:

- rendering or UI, what the player sees;
- client-server sync: LAN guests, a dedicated server, packets;
- when or in what order something happens compared with vanilla, such as a hook's firing point.

If your PR changes or removes something mods already use, say so in the PR: mods built against the old API stop
building. A new item's `///` doc is its reference.

## A catalog point and the wasm fixture

A new or changed catalog point is proven by the wasm fixture: real wasm mods (`crates/horizon-mod-wasm/tests/fixture`,
`library-fixture` and `groups`) built with `cargo horizon-mod build --wasm` and loaded by the real loader. The
`wasm-mods` CI job does all of it on pushes to main and on manual dispatch; it does not run on pull requests. Main
pushes coalesce: a started run finishes and only the newest waiting push runs next, so the newest finished main run
proves every commit before it. A red is attributed by reading the error, with a manual dispatch on a suspect commit
(through a branch pointing at it) when needed. A maintainer dispatches it on a PR branch that changes what it builds. That job is the answer; you don't
need a local build.

To run it yourself, on Linux or Windows, follow the steps of `.github/workflows/wasm-mods.yml` from the repository
root; the tool installs its pinned toolchain through rustup. To check one change, build only the member that uses
it and run its test. The build rewrites files in the fixture folders; don't commit them.
