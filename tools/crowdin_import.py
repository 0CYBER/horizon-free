#!/usr/bin/env python3
"""Write Crowdin's export into the game's lang files and the review record (docs/translations.md).

Crowdin is where Horizon's own text is translated and proofread; this script is the bridge between
its export and the two things a reviewer cares about: the lang file the game reads, and
`tools/lang_review/game/`, the record of what a person who speaks the language checked.

Called by `.github/workflows/crowdin.yml` with two exports of the same project:

    python3 tools/crowdin_import.py --translations l10n/game --approved l10n/game-approved

`--translations` holds every translation in the project, `--approved` the same languages carrying
only proofread strings (`export_only_approved`, `.github/crowdin-approved.yml`). A key is recorded
as reviewed exactly when the approved export has it with the text this run wrote: that is the rule
docs/translations.md states, and it is why an edit a translator makes without a proofread takes the
mark off again by itself.

Crowdin checks neither the key set nor the placeholders against the English text, so both are done
here: a translation whose `%s`/`%1$s` fields differ is dropped and reported, and so is one that is
the English sentence itself (Crowdin fills an untranslated string with its source unless the export
skips it). Either way the language keeps the text it had.

`--stage` is the other direction, once: it copies the lang files into the staging folder under the
names Crowdin's export uses, so the first `upload translations` seeds the project with the strings the
repository already has instead of an empty folder.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:  # imported as `tools.crowdin_import` by the tests, run as a script by the workflow
    from tools import lang_coverage
except ModuleNotFoundError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import lang_coverage

ROOT = Path(__file__).resolve().parents[1]
LANG = ROOT / "crates/horizon-client/assets/horizon/lang"
REVIEW = ROOT / "tools/lang_review/game"

# The game's lang code (vanilla's: `fr_fr`, `zh_cn`, `lmo`) and the Crowdin locale that holds it,
# as Crowdin names it in `%locale_with_underscore%` (`de-DE` renders as `de_DE`) — the name it writes
# into the staging folder, and the name this file's lang file carries. The bare language id comes
# second as a fallback (`de`), and a language Crowdin renames without its region still resolves
# through its language subtag when only one code claims that subtag; `pt` and `sr` do not, so their
# two codes are named exactly.
LOCALES: dict[str, tuple[str, ...]] = {
    "af_za": ("af-ZA", "af"),
    "ar_sa": ("ar-SA", "ar"),
    "az_az": ("az-AZ", "az"),
    "ba_ru": ("ba-RU", "ba"),
    "be_by": ("be-BY", "be"),
    "bg_bg": ("bg-BG", "bg"),
    "br_fr": ("br-FR",),
    "bs_ba": ("bs-BA", "bs"),
    "ca_es": ("ca-ES", "ca"),
    "cs_cz": ("cs-CZ", "cs"),
    "cv_cu": ("cv-CU", "cv"),
    "cy_gb": ("cy-GB", "cy"),
    "da_dk": ("da-DK", "da"),
    "de_de": ("de-DE", "de"),
    "el_gr": ("el-GR", "el"),
    "eo_uy": ("eo-UY", "eo"),
    "es_es": ("es-ES", "es"),
    "et_ee": ("et-EE", "et"),
    "eu_es": ("eu-ES", "eu"),
    "fa_ir": ("fa-IR", "fa"),
    "fi_fi": ("fi-FI", "fi"),
    "fil_ph": ("fil-PH", "fil"),
    "fo_fo": ("fo-FO", "fo"),
    "fr_fr": ("fr-FR", "fr"),
    "fur_it": ("fur-IT",),
    "fy_nl": ("fy-NL",),
    "ga_ie": ("ga-IE", "ga"),
    "gd_gb": ("gd-GB", "gd"),
    "gl_es": ("gl-ES", "gl"),
    "haw_us": ("haw-US", "haw"),
    "he_il": ("he-IL", "he"),
    "hi_in": ("hi-IN", "hi"),
    "hr_hr": ("hr-HR", "hr"),
    "hu_hu": ("hu-HU", "hu"),
    "hy_am": ("hy-AM", "hy"),
    "id_id": ("id-ID", "id"),
    "ig_ng": ("ig-NG", "ig"),
    "is_is": ("is-IS", "is"),
    "it_it": ("it-IT", "it"),
    "ja_jp": ("ja-JP", "ja"),
    "ka_ge": ("ka-GE", "ka"),
    "kk_kz": ("kk-KZ", "kk"),
    "kn_in": ("kn-IN", "kn"),
    "ko_kr": ("ko-KR", "ko"),
    "ky_kg": ("ky-KG", "ky"),
    "la_la": ("la-LA", "la"),
    "lb_lu": ("lb-LU", "lb"),
    "li_li": ("li-LI", "li"),
    "lo_la": ("lo-LA", "lo"),
    "lt_lt": ("lt-LT", "lt"),
    "lv_lv": ("lv-LV", "lv"),
    "mk_mk": ("mk-MK", "mk"),
    "mn_mn": ("mn-MN", "mn"),
    "ms_my": ("ms-MY", "ms"),
    "mt_mt": ("mt-MT", "mt"),
    "nl_nl": ("nl-NL", "nl"),
    "no_no": ("no-NO", "no", "nb", "nb-NO"),
    "oc_fr": ("oc-FR", "oc"),
    "pl_pl": ("pl-PL", "pl"),
    "pt_br": ("pt-BR",),
    "pt_pt": ("pt-PT",),
    "ro_ro": ("ro-RO", "ro"),
    "ru_ru": ("ru-RU", "ru"),
    "sah_sah": ("sah-SAH", "sah"),
    "se_no": ("se-NO", "se"),
    "sk_sk": ("sk-SK", "sk"),
    "sl_si": ("sl-SI", "sl"),
    "so_so": ("so-SO", "so"),
    "sq_al": ("sq-AL", "sq"),
    "sr_cs": ("sr-CS",),
    "sr_sp": ("sr-SP", "sr"),
    "sv_se": ("sv-SE",),
    "ta_in": ("ta-IN", "ta"),
    "th_th": ("th-TH", "th"),
    "tr_tr": ("tr-TR", "tr"),
    "tt_ru": ("tt-RU", "tt"),
    "uk_ua": ("uk-UA", "uk"),
    "ur_pk": ("ur-PK", "ur"),
    "uz_uz": ("uz-UZ", "uz"),
    "vec_it": ("vec-IT", "vec"),
    "vi_vn": ("vi-VN", "vi"),
    "yi_de": ("yi-DE", "yi"),
    "yo_ng": ("yo-NG", "yo"),
    "zh_cn": ("zh-CN",),
}

# Vanilla names these, Crowdin has no such language: the file stays hand-kept and the sync leaves it
# alone. A lang file that is in neither table is an error, so a new language cannot go unnoticed.
NOT_IN_CROWDIN = {
    "lmo": "Lombard is not one of Crowdin's languages",
    "szl": "Silesian is not one of Crowdin's languages",
}


def key(locale: str) -> str:
    """Crowdin's way of writing a locale (`pt_BR`, `sv-SE`, `de`) in one lookup form."""
    return locale.strip().replace("_", "-").lower()


def index() -> dict[str, str]:
    """Every accepted Crowdin locale, in `key()` form, to the game's code.

    A bare language subtag is accepted when only one code claims it: Crowdin names many languages
    without a region (`de`, `uk`) and `%locale_with_underscore%` may still render one with it
    (`de_DE`, `uk_UA`), so `language()` falls back to the subtag as well.
    """
    table: dict[str, str] = {}
    subtags: dict[str, set[str]] = {}
    for code, locales in LOCALES.items():
        for locale in locales:
            table[key(locale)] = code
            subtags.setdefault(key(locale).split("-")[0], set()).add(code)
    for subtag, codes in subtags.items():
        if len(codes) == 1 and subtag not in table:
            table[subtag] = next(iter(codes))
    return table


def language(locale: str, table: dict[str, str]) -> str | None:
    """The game's code for an exported filename, or None when the table does not name it."""
    exact = key(locale)
    if exact in table:
        return table[exact]
    # `uk_UA` for a language the table holds as `uk`: the subtag is only in the table when one code
    # claims it, so an ambiguous one (`pt`, `sr` without a region) stays unnamed.
    return table.get(exact.split("-")[0])


def newline_of(path: Path) -> str:
    """The newline the file is checked out with, so a write keeps it (`cargo fmt` never touches these)."""
    try:
        return "\r\n" if b"\r\n" in path.read_bytes() else "\n"
    except OSError:
        return "\n"


def render(entries: dict[str, str], newline: str) -> str:
    """One `"key": "value"` entry per line and two spaces of indentation, as the lang files are kept."""
    body = ",\n".join(
        f"  {json.dumps(name, ensure_ascii=False)}: {json.dumps(text, ensure_ascii=False)}"
        for name, text in entries.items())
    return "{" + newline + body.replace("\n", newline) + newline + "}" + newline


def placeholders_kept(english: str, text: str) -> bool:
    """A translation keeps every field the code fills in (`%s`, `%1$s`) and adds none."""
    return lang_coverage.placeholders(english) == lang_coverage.placeholders(text)


def merge(existing: dict[str, str], incoming: dict[str, str],
          order: list[str]) -> tuple[dict[str, str], list[str], int]:
    """The file's new text: the order it is in today, then new keys in the English file's order.

    A key English no longer has keeps its text: `lang_coverage.py` reports it as stale and a
    reviewer still wants the sentence. Returns the entries, the added keys and how many texts
    changed.
    """
    merged: dict[str, str] = {}
    changed = 0
    for name, text in existing.items():
        if name in incoming:
            changed += incoming[name] != text
            merged[name] = incoming[name]
        else:
            merged[name] = text
    added = [name for name in order if name in incoming and name not in merged]
    for name in added:
        merged[name] = incoming[name]
    return merged, added, changed


def read_keys(path: Path) -> list[str]:
    """The keys the sync recorded in one language last run (a missing or broken file means none)."""
    try:
        keys = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return []
    return [key for key in keys if isinstance(key, str)] if isinstance(keys, list) else []


def write_keys(path: Path, keys: list[str]) -> None:
    if keys:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes((json.dumps(keys, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
    elif path.exists():
        path.unlink()


def write_record(path: Path, entries: dict[str, str], order: list[str]) -> None:
    """The record in the lang file's order, one entry per line; no file when nothing is checked."""
    ordered = {name: entries[name] for name in order if name in entries}
    if ordered:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes((json.dumps(ordered, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
    elif path.exists():
        path.unlink()


def sync_language(code: str, exported: Path, approved: Path, lang: Path, review: Path,
                  english: dict[str, str], write: bool) -> dict:
    """One language: its exported text into `lang/<code>.json`, its proofread keys into the record."""
    target = lang / f"{code}.json"
    existing = lang_coverage.LangFile(target).entries
    incoming: dict[str, str] = {}
    dropped: list[str] = []
    for name, text in lang_coverage.LangFile(exported).entries.items():
        if not text.strip() or name not in english:
            continue  # a blank value is a gap, and a key English lost is not this file's to write
        if not placeholders_kept(english[name], text):
            dropped.append(name)
        elif text == english[name] and existing.get(name) != text:
            # Crowdin fills an untranslated string with its English source unless the export skips
            # them (`skip_untranslated_strings`); writing that would replace a translation with
            # English, so it stays out. A language whose word really is the English one keeps it,
            # because its file already holds that text.
            dropped.append(name)
        else:
            incoming[name] = text
    merged, added, changed = merge(existing, incoming, list(english))

    checked: list[str] = []
    if approved.exists():
        proofread = lang_coverage.LangFile(approved).entries
        checked = [name for name in merged if proofread.get(name) == merged[name]]

    record = review / f"{code}.json"
    state = review.parent / "crowdin" / f"{code}.json"
    ours = set(read_keys(state))
    before = lang_coverage.reviewed(record)
    if write:
        # Bytes, not text: `write_text` would translate the "\n" of a CRLF file into a second "\r".
        target.write_bytes(render(merged, newline_of(target)).encode("utf-8"))
        # The record is rewritten for the keys the sync wrote last time, so a withdrawn approval
        # takes its mark back, and left alone for every entry a person put there by hand.
        kept = {name: text for name, text in before.items()
                if name not in ours and merged.get(name) == text}
        kept.update({name: merged[name] for name in checked})
        write_record(record, kept, list(merged))
        write_keys(state, checked)
    return {
        "code": code,
        "keys": len(existing),
        "written": len(merged),
        "new": len(added),
        "changed": changed,
        "checked": len(checked),
        "kept": sum(1 for name in before if name not in ours),
        "dropped": dropped,
    }


def sync(translations: Path, approved: Path, lang: Path = LANG, review: Path = REVIEW,
         write: bool = True) -> tuple[list[dict], list[str]]:
    """Every exported language under `translations`, and the filenames the table does not name."""
    table = index()
    english_file = lang_coverage.LangFile(lang / "en_us.json")
    if english_file.problem:
        raise SystemExit(f"{lang / 'en_us.json'}: {english_file.problem}")
    rows, unmapped = [], []
    for exported in sorted(translations.glob("*.json")):
        code = language(exported.stem, table)
        if code is None:
            unmapped.append(exported.stem)
            continue
        rows.append(sync_language(code, exported, approved / exported.name, lang, review,
                                  english_file.entries, write))
    return rows, unmapped


def stage(lang: Path, folder: Path, write: bool = True) -> tuple[list[dict], list[str]]:
    """The lang files under the names Crowdin's export uses, for the first `upload translations`.

    Crowdin reads translations from the folder the `translation` pattern in `crowdin.yml` names, which
    is a staging folder and empty on a fresh checkout. Without this step the first run would seed the
    project with nothing, and the download after it would carry no translation at all.
    """
    rows: list[dict] = []
    hand_kept: list[str] = []
    for path in sorted(lang.glob("*.json")):
        code = path.stem
        if code == "en_us":
            continue
        if code not in LOCALES:
            hand_kept.append(code)  # `lmo`, `szl`: no Crowdin language to upload them to
            continue
        entries = lang_coverage.LangFile(path).entries
        name = LOCALES[code][0].replace("-", "_") + ".json"
        if write:
            folder.mkdir(parents=True, exist_ok=True)
            (folder / name).write_bytes(render(entries, "\n").encode("utf-8"))
        rows.append({"code": code, "file": name, "keys": len(entries)})
    return rows, hand_kept


def check(lang: Path = LANG) -> list[str]:
    """The table and the lang folder against each other: both directions have to line up."""
    problems = [f"no Crowdin language for lang file {path.stem}: add it to tools/crowdin_import.py"
                for path in sorted(lang.glob("*.json"))
                if path.stem != "en_us" and path.stem not in LOCALES
                and path.stem not in NOT_IN_CROWDIN]
    problems += [f"lang file missing for Crowdin language {code}: {locales[0]}"
                 for code, locales in sorted(LOCALES.items())
                 if not (lang / f"{code}.json").exists()]
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--translations", type=Path, default=ROOT / "l10n/game",
                        help="Crowdin's export of every translation, one file per language")
    parser.add_argument("--approved", type=Path, default=ROOT / "l10n/game-approved",
                        help="the same export with only proofread strings (export_only_approved)")
    parser.add_argument("--lang", type=Path, default=LANG, help="the game's lang folder")
    parser.add_argument("--review", type=Path, default=REVIEW, help="the game's review record")
    parser.add_argument("--dry-run", action="store_true",
                        help="print what would change and write nothing")
    parser.add_argument("--stage", action="store_true",
                        help="only copy the lang files into the folder Crowdin uploads translations "
                             "from (the first run's seed), then exit")
    parser.add_argument("--stage-dir", type=Path, default=ROOT / "l10n/game",
                        help="with --stage: the staging folder Crowdin's `translation` pattern names")
    parser.add_argument("--check", action="store_true",
                        help="only check that the table covers every lang file, then exit")
    arguments = parser.parse_args(argv)

    if arguments.stage:
        rows, hand_kept = stage(arguments.lang, arguments.stage_dir, write=not arguments.dry_run)
        print(f"  {'code':<10}{'keys':>6}  file")
        for row in rows:
            print(f"  {row['code']:<10}{row['keys']:>6}  {row['file']}")
        print(f"\n{len(rows)} languages, {sum(row['keys'] for row in rows)} strings staged into "
              f"{arguments.stage_dir}; hand-kept: {', '.join(sorted(hand_kept))}")
        return 0

    if arguments.check:
        problems = check(arguments.lang)
        for problem in problems:
            print(problem, file=sys.stderr)
        kept = ", ".join(sorted(NOT_IN_CROWDIN))
        print(f"Crowdin languages: {len(LOCALES)} mapped, "
              f"{len(list(arguments.lang.glob('*.json'))) - 1} lang files, "
              f"hand-kept: {kept}, {len(problems)} problems")
        return 1 if problems else 0

    if not arguments.translations.is_dir():
        print(f"{arguments.translations} does not exist: nothing to import", file=sys.stderr)
        return 1

    rows, unmapped = sync(arguments.translations, arguments.approved, arguments.lang,
                          arguments.review, write=not arguments.dry_run)
    if not rows:
        print(f"{arguments.translations}: no exported language the table names", file=sys.stderr)
        return 1

    print(f"  {'code':<10}{'keys':>6}{'written':>9}{'new':>6}{'changed':>9}{'checked':>9}")
    for row in rows:
        print(f"  {row['code']:<10}{row['keys']:>6}{row['written']:>9}{row['new']:>6}"
              f"{row['changed']:>9}{row['checked']:>9}")
    print(f"\n{len(rows)} languages, {sum(row['new'] for row in rows)} new keys, "
          f"{sum(row['changed'] for row in rows)} changed texts, "
          f"{sum(row['checked'] for row in rows)} recorded as reviewed")
    for row in rows:
        if row["dropped"]:
            print(f"\n{row['code']}: {len(row['dropped'])} translations change the placeholders of "
                  f"the English text and were not written: {', '.join(row['dropped'][:5])}",
                  file=sys.stderr)
    if unmapped:
        print(f"\nnot a Crowdin language this table knows: {', '.join(unmapped)} — add it to "
              f"LOCALES in tools/crowdin_import.py and run again", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
