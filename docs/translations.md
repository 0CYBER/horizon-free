# Translations

Horizon's own text (its menus, settings and the launcher) is translated into about 85 languages. Vanilla's text
comes from the player's own Minecraft install and is not translated here. Most of Horizon's translations were
written by machine, and players reported some of them as wrong, so the project decided on 2026-11-09 that every
machine-written string carries an **unreviewed** mark until a person who speaks the language has read it. Players see
no difference: the mark lives in a file the game and the launcher never read.

Translating and checking the game's text happens in [Crowdin](https://crowdin.com/project/horizon-client): a
translator writes a string there, a proofreader approves it, and the nightly sync turns that approval into the
reviewed mark. This page is the repository side of that — what the sync writes, what a mark is worth, and what is
still hand-kept.

## Where the text lives

| | Files | Keys | Read by |
|---|---|---|---|
| Game | `crates/horizon-client/assets/horizon/lang/<code>.json` (`fr_fr`, `de_de`...) | `horizon.options.foo` style; `en_us.json` is the reference | `crates/horizon-client-core/src/lang.rs` (`Text::tr`, `lang::get`); a missing key falls back to English |
| A crate's own text | `crates/<name>/assets/lang/<code>.json`, the same shape | the crate's own keys, never one of the engine's | the same, once the crate registers them ([layers.md](layers.md#lang-files-horizon_platformlang)) |
| Launcher | `launcher/lang/<code>.json` (`fr`, `de`, `pt-br`...) | the English sentence itself | `launcher/src/i18n.rs` (`tr("…")`, `trf("… {arg} …")`) |
| Review record | `tools/lang_review/game/<code>.json`, `tools/lang_review/<crate name>/<code>.json`, `tools/lang_review/launcher/<code>.json` | the same keys, with the text a person checked | `tools/lang_coverage.py` only |

An entry is **reviewed** when the review record holds its key with exactly its present text. Anything else is
unreviewed: an entry the record does not hold, and one whose text changed since a person checked it (so a later
machine edit makes it unreviewed again). New strings need no extra step: added by machine, they start unreviewed.

## Crowdin

[The project](https://crowdin.com/project/horizon-client) carries the game's text only. `en_us.json` is its source
file, and `.github/workflows/crowdin.yml` is the whole sync:

- a push to `main` that changes `en_us.json` uploads the English text, so a translator sees a new string the day it
  lands;
- the nightly run (and a run by hand, which says which half to do) downloads two exports and opens one pull
  request, `translations from Crowdin`: every translation into `l10n/game`, and the strings a proofreader approved
  into `l10n/game-approved`. Neither folder is committed — `tools/crowdin_import.py` reads them, writes the lang
  files and the review record, and the workflow commits the result.

**An approval in Crowdin is what clears the mark.** The import records a key as reviewed exactly when the approved
export carries the text the lang file now has, so a translator's edit that nobody proofread takes the mark off again
by itself, and no one has to run `--mark-reviewed` for Crowdin's work. A withdrawn approval takes it off too: the
record of what the sync wrote last time lives in `tools/lang_review/crowdin/<code>.json`, so the sync replaces its
own entries and leaves every entry a person put there by hand alone. The tool's own `--mark-reviewed` stays for
wording that arrives through [#translations](https://discord.gg/playhorizon) instead.

Two things the project does not check, so the import does:

- **Placeholders.** A translation whose `%s`/`%1$s` fields differ from the English text is dropped and reported; the
  language keeps the text it had.
- **Untranslated strings.** Crowdin writes the English sentence into a string nobody translated unless the export
  skips them; both downloads ask it to skip (`skip_untranslated_strings`), and the import drops a value that is the
  English text unless the lang file already holds exactly that. A language whose word really is the English one
  keeps it.
- **Key set.** A key the English file no longer has is not imported, and a key English gained is untranslated until
  someone writes it — the game falls back to English meanwhile.

The first run is manual, with **upload_translations** ticked: `tools/crowdin_import.py --stage` copies the lang files
into `l10n/game-staged` — the folder the workflow's `upload_translations_args` names with `-t` — so the strings already in the repository
reach the project instead of an empty staging folder, and the download after it has something to compare with. The
download writes into `l10n/game` instead, one folder per direction, so a language the download carries nothing for
cannot keep the staged text. After that only what changed moves. Enabling the workflow takes the repository variable `CROWDIN_ENABLED` and the secrets `CROWDIN_PROJECT_ID`
and `CROWDIN_PERSONAL_TOKEN`; without them it stays switched off.

Not in Crowdin, and still hand-edited: the launcher's text (its keys are the English sentences and its language list
is fixed in code — a later step), and `lmo` and `szl`, for which Crowdin has no Lombard or Silesian.

## What is left to review

```sh
python3 tools/lang_coverage.py --summary                   # one line per file, "unreviewed N" at the end
python3 tools/lang_coverage.py --unreviewed --lang fr_fr fr # every unreviewed entry, its English and its text
python3 tools/lang_coverage.py --json                      # the same, for a script ("unreviewed" per file)
```

## Reviewing a language

Review happens in **Crowdin**, and [#translations on Horizon's Discord](https://discord.gg/playhorizon) is where you
get in and where a single wrong string is still reported.

1. **Review a whole language**: ask in #translations for a Crowdin account and the proofreader role for your
   language. The untranslated strings, and the ones a machine wrote, are the work list; the ones you approve are
   what the next pull request records as reviewed.
2. **Report a wrong string**: post the language, where you saw it (screen, setting, launcher page), the current text
   and the better text. Anyone can do this; you don't need GitHub.
3. **A report can also land as a pull request**: a maintainer or agent changes the lang file with your wording and
   records the checked entries as reviewed. **The same wording has to be entered in Crowdin as well** (as a
   translation, approved by a proofreader): the sync writes the game's text from the project, so a change that only
   exists in the repository is replaced by the project's text at the next run. The repository-only path stays for
   what Crowdin does not carry — the launcher, `lmo` and `szl`:

   ```sh
   python3 tools/lang_coverage.py --lang fr_fr --mark-reviewed horizon.options.foo horizon.options.bar
   python3 tools/lang_coverage.py --lang fr --mark-reviewed --keys-file checked.txt   # one key per line
   ```

   `--mark-reviewed` records the entries' present text, so change the text first, then mark it. It also drops
   entries the file no longer has. A code that is both a game and a launcher file (`lmo`, `szl`) needs
   `--game-only` or `--launcher-only`. Pull requests that only change translations take no label.

## Rules

- **Only a person who speaks the language clears a mark**: a Crowdin proofreader's approval, or a wording quoted in
  the pull request from a #translations report. Never run `--mark-reviewed` on machine-written text, and never on
  text an AI tool rewrote, even when it reads well.
- A new launcher string still needs an entry in every launcher lang file (`launcher/src/i18n/tests.rs` checks it),
  and a new game string falls back to English where a language lacks it. A machine translation is fine for both: it
  starts unreviewed by itself.
- Keep placeholders as they are (`%s`, `%1$s` in the game, `{name}` in the launcher): `lang_coverage.py` reports a
  changed one, and the Crowdin import refuses to write it.

## How the first record was made

On 2026-11-09 every entry of every lang file was traced to the commit that last changed it (`git blame -M`). An entry
counts as written by a person, and went into the record, only when that commit:

- was not made under an agent's identity (`Ludovic Marie`, which his agents commit as, `Horizon agent`, `Claude`,
  bots) and is not a merge,
- has no AI co-author or AI mention in its message (Claude, Codex, GPT, Copilot, Cursor, Gemini, DeepSeek,
  opencode...),
- changed at most three non-English lang files,
- and is by an author whose lang changes, over the whole history, cover at most two languages (a person writes the
  one or two languages they speak; an author covering ten is a tool).

That kept 1,917 entries by nine contributors (Czech, German, Japanese, Brazilian Portuguese, Russian and Turkish).
The rule errs towards unreviewed: a wrong "unreviewed" costs a reviewer a glance, a wrong "reviewed" hides a bad
string. Everything else, about 197,000 entries, starts unreviewed.
