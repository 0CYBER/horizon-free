#!/usr/bin/env python3
"""How much of Horizon's own text each language carries, and where it still says English.

`crates/horizon-client/assets/horizon/lang/en_us.json` is the reference: every key the
game asks for. A language that misses a key falls back to English at runtime
(`Language::load` in `crates/horizon-client-core/src/lang.rs` layers `en_us` under it), so
a gap is not an error — it is an English sentence on a translated screen. That file's
tests catch stale keys, repeated keys, empty values and changed placeholders, but
deliberately not gaps, because a language is allowed to lag. This tool reports them.

A crate that ships its own text keeps it in `crates/<name>/assets/lang/<code>.json`, the same
shape with its own `en_us.json` reference (docs/layers.md, "Lang files"); each such folder is
reported beside the game's, and its review record is `tools/lang_review/<name>/`.

`launcher/lang/<code>.json` is the launcher's: its keys are the English sentence itself,
so a file is complete by construction (`launcher/src/i18n/tests.rs` checks every `tr("…")`
the code asks for). What it can still hold is a value identical to its English key, and a
`{hole}` the value dropped or added.

Most translations were written by machine. A translated entry is *unreviewed* until a person
who speaks the language has read it: `tools/lang_review/{game,launcher}/<code>.json` records
the text a person wrote or checked, key for key, and any other entry, or one whose text changed
since, counts as unreviewed. Players see no difference; the record is for reviewers
(docs/translations.md).

    python3 tools/lang_coverage.py                 # every file, both folders, most complete first
    python3 tools/lang_coverage.py --lang es_es es # only those file stems
    python3 tools/lang_coverage.py --game-only
    python3 tools/lang_coverage.py --summary       # one line per file, for a pull request
    python3 tools/lang_coverage.py --show-keys     # the keys behind every count
    python3 tools/lang_coverage.py --summary --show-keys  # the summary, then the keys behind it
    python3 tools/lang_coverage.py --json          # the same, for a script
    python3 tools/lang_coverage.py --fail-under 95 # exit 1 under that cover (for CI, later)
    python3 tools/lang_coverage.py --unreviewed --lang fr_fr        # what a French reviewer reads
    python3 tools/lang_coverage.py --lang fr_fr --mark-reviewed KEY # a person checked KEY's text
    python3 tools/lang_coverage.py --lang fr --mark-reviewed --keys-file checked.txt

It needs no `.local/`, and writes nothing but the review record, and that only with
`--mark-reviewed`.
"""

import argparse
import json
import re
import sys
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GAME = ROOT / "crates/horizon-client/assets/horizon/lang"
CRATES = ROOT / "crates"
LAUNCHER = ROOT / "launcher/lang"
REVIEW = ROOT / "tools/lang_review"
REFERENCE = "en_us.json"


def shown(path: Path) -> str:
    """A path relative to the repository when it is inside it, portable separators either way."""
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return str(path)


USIZE_MAX = 2**64 - 1


def usize(digits: str) -> int | None:
    """Rust's `digits.parse::<usize>().ok()`, without building an int from thousands of digits."""
    significant = digits.lstrip("0") or "0"
    if len(significant) > len(str(USIZE_MAX)):
        return None
    value = int(significant)
    return value if value <= USIZE_MAX else None


def translation_pieces(template: str) -> list[str | int] | None:
    """The game's `translation_pieces` (`crates/horizon-client-core/src/text.rs`), the parser its
    formatter runs: literal text (a str, `%%` already a percent sign) and the argument index each
    field takes (an int; `%0$s` asks for USIZE_MAX), in order. `%s` takes the next argument, `%n$s`
    the n-th; a percent sign that starts no conversion ("100% sure", "%.2f") is text. None where the
    game shows the template as written whatever its arguments: another letter such as `%d` or `%q`,
    a `%` at the end, `%1$` or `%1$%`. `tests/fixtures/translation_templates.json` pins this
    against the game's own test."""
    pieces: list[str | int] = []
    literal: list[str] = []
    following = 0
    i = 0
    while i < len(template):
        if template[i] != "%":
            literal.append(template[i])
            i += 1
            continue
        j = digits = i + 1
        while j < len(template) and template[j] in "0123456789":
            j += 1
        number = None
        if j > digits and j < len(template) and template[j] == "$":
            number = usize(template[digits:j])
            j += 1
        else:
            j = digits
        conversion = template[j] if j < len(template) else None
        if conversion == "%" and number is None:
            literal.append("%")
            i = j + 1
        elif conversion is not None and conversion != "%" and not (
                conversion.isascii() and conversion.isalpha()):
            literal.append("%")
            i += 1
        elif conversion == "s":
            if number is None:
                index = following
                following += 1
            else:
                index = (number - 1) % (USIZE_MAX + 1)  # Rust's `wrapping_sub(1)`
            if literal:
                pieces.append("".join(literal))
                literal = []
            pieces.append(index)
            i = j + 1
        else:
            return None
    if literal:
        pieces.append("".join(literal))
    return pieces


def format_translation(template: str, args: list[str]) -> str:
    """The game's `format_translation` on plain text: the template as written when its parser
    refuses it or it asks for an argument the call lacks."""
    pieces = translation_pieces(template)
    if pieces is None or any(isinstance(piece, int) and piece >= len(args) for piece in pieces):
        return template
    return "".join(args[piece] if isinstance(piece, int) else piece for piece in pieces)


def template(text: str) -> tuple[list[int], str] | None:
    """How the game reads `text` as a template: the argument indices it fills in, once per use and
    sorted, and its literal text; None where it shows the template as written whatever the
    arguments (`%0$s` included: no call has an argument at USIZE_MAX). `lang.rs`'s bundled-file
    test computes the same from the game's parser."""
    pieces = translation_pieces(text)
    if pieces is None or USIZE_MAX in pieces:
        return None
    return (sorted(piece for piece in pieces if isinstance(piece, int)),
            "".join(piece for piece in pieces if isinstance(piece, str)))


UNFORMATTABLE_FILE = ROOT / "tools/lang_unformattable.json"


def unformattable(body: str | None = None) -> set[str]:
    """The keys whose English text the game cannot format: they are read as plain text, never
    formatted, so their translations only have to fill in no argument. The list may only shrink:
    `tests/test_lang_coverage.py` and `lang.rs` pin its keys, and `lang.rs` fails on a listed key
    whose English formats."""
    return set(json.loads(UNFORMATTABLE_FILE.read_text(encoding="utf-8") if body is None else body))


def formats_like(key: str, english: str, text: str, plain: set[str] | None = None) -> bool:
    """A translation the game can format, filling in the same arguments as the English text, and
    still showing a percent sign where English writes one as `%%`."""
    want, got = template(english), template(text)
    if key in (unformattable() if plain is None else plain):
        return (want or ([], ""))[0] == (got or ([], ""))[0]
    if got is None or want is None or got[0] != want[0]:
        return False
    return "%%" not in english or "%" in got[1]


# Unicode's bidirectional embedding, override and isolate controls: one a translation adds can
# turn the rest of a line around.
BIDI_CONTROLS = frozenset("\u202a\u202b\u202c\u202d\u202e\u2066\u2067\u2068\u2069")


def stray_controls(english: str, text: str) -> list[str]:
    """The control characters (C0, DEL, C1) and bidi controls `text` must not carry: a line break
    is always allowed, a tab and a bidi control only where its English text has one, any other
    control character never. `lang.rs`'s bundled-file test applies the same rule."""
    return sorted({c for c in text if c != "\n" and (
        c not in english if c == "\t" or c in BIDI_CONTROLS else unicodedata.category(c) == "Cc")})


# What a translated string may carry beyond its English text (docs/translations.md, "Rules"). The
# text comes from an open Crowdin project, so the limits assume a hostile writer. `lang.rs`'s
# bundled-file test applies the same rules; `tests/fixtures/translation_text_rules.json` pins both.
LENGTH_FACTOR = 4  # at most this many times the English length, in characters...
LENGTH_SLACK = 80  # ...or the English length plus this many, whichever is more
MAX_RUN = 60  # the longest run without a break, unless the English text has a longer one
MAX_MARKS = 4  # the most combining marks in a row, unless the English text has more

# Invisible characters a translation may carry only where its English text has the same one:
# zero-width space and word joiner, line and paragraph separators, the byte order mark, the soft
# hyphen, the combining grapheme joiner, the Mongolian vowel separator, the Hangul fillers and the
# interlinear annotation controls. Tag characters, noncharacters and private-use characters are
# checked by range (`invisible`).
INVISIBLE = frozenset("\u200b\u2060\u2028\u2029\ufeff\u00ad\u034f\u180e"
                      "\u115f\u1160\u3164\uffa0\ufff9\ufffa\ufffb")
# Openers of a bidi embedding or override (closed by U+202C) and of an isolate (closed by U+2069).
EMBEDDINGS, ISOLATES = frozenset("\u202a\u202b\u202d\u202e"), frozenset("\u2066\u2067\u2068")
# Character tables both sides read, written by tools/gen_lang_unicode_tables.py from the Unicode
# Character Database: "spaceless" letters of scripts written without spaces between words, which
# the game's wrapper cuts where a line overflows, so they don't count towards a token;
# "combining" marks; and "transparent" characters (format characters, emoji modifiers and other
# grapheme extenders) that neither count as a mark nor end a row of them.
UNICODE_TABLES_FILE = ROOT / "tools/lang_unicode_tables.json"
UNICODE_TABLES = ("spaceless", "combining", "transparent")
_TABLES: dict[str, list[tuple[int, int]]] = {}


def _in_table(name: str, c: str) -> bool:
    """`c` in one of the sorted, disjoint range lists of tools/lang_unicode_tables.json."""
    if not _TABLES:
        body = json.loads(UNICODE_TABLES_FILE.read_text(encoding="utf-8"))
        _TABLES.update({key: [tuple(r) for r in body[key]] for key in UNICODE_TABLES})
    table, o = _TABLES[name], ord(c)
    low, high = 0, len(table)
    while low < high:
        middle = (low + high) // 2
        if table[middle][1] < o:
            low = middle + 1
        else:
            high = middle
    return low < len(table) and table[low][0] <= o


def invisible(c: str) -> bool:
    o = ord(c)
    return (c in INVISIBLE or 0xE0000 <= o <= 0xE007F  # tags
            or 0xFDD0 <= o <= 0xFDEF or o & 0xFFFE == 0xFFFE  # noncharacters
            or 0xE000 <= o <= 0xF8FF or 0xF0000 <= o <= 0xFFFFD or 0x100000 <= o <= 0x10FFFD)


def variation_selector(c: str) -> bool:
    o = ord(c)
    return (0xFE00 <= o <= 0xFE0F or 0xE0100 <= o <= 0xE01EF
            or 0x180B <= o <= 0x180D or o == 0x180F)  # Mongolian free variation selectors


def formatting_codes(text: str) -> list[str]:
    """The legacy codes in `text`, in order: a `§` takes the character after it, whatever it is,
    and the game draws neither (`Text::resolve_legacy`); an ASCII code is matched without case.
    A `§` at the end is a code of its own ("")."""
    found, i = [], 0
    while (at := text.find("§", i)) != -1:
        code = text[at + 1:at + 2]
        found.append(code.lower() if code.isascii() else code)  # to_ascii_lowercase
        i = at + 2
    return found


def longest_run(text: str) -> int:
    """The most characters one token holds that the game's wrapper cannot break inside: a token is
    what lies between spaces and line breaks (`Font::split_lines` breaks there only), and letters of
    a script written without spaces do not count, since a line of them is cut where it overflows."""
    return max((sum(1 for c in token if not _in_table("spaceless", c))
                for line in text.split("\n") for token in line.split(" ")), default=0)


def most_marks(text: str) -> int:
    """The most combining marks in a row; a transparent character between them (a joiner, a
    direction mark, an emoji modifier) neither counts nor ends the row."""
    most = current = 0
    for c in text:
        if _in_table("transparent", c):
            continue
        current = current + 1 if _in_table("combining", c) else 0
        most = max(most, current)
    return most


def bidi_balanced(english: str, text: str) -> bool:
    """Every bidi embedding, override and isolate `text` opens it closes, nested, and it opens no
    more of each than its English text does."""
    if any(text.count(c) > english.count(c) for c in EMBEDDINGS | ISOLATES):
        return False
    open_: list[str] = []
    for c in text:
        if c in EMBEDDINGS or c in ISOLATES:
            open_.append(c)
        elif c in "\u202c\u2069":
            closes = EMBEDDINGS if c == "\u202c" else ISOLATES
            if not open_ or open_.pop() not in closes:
                return False
    return not open_


def text_problem(english: str, text: str) -> str | None:
    """The first rule `text`, a translation of `english`, breaks, by its name in
    `tests/fixtures/translation_text_rules.json`, or None."""
    if stray_controls(english, text):
        return "control"
    for i, c in enumerate(text):
        if invisible(c) and c not in english:
            return "invisible"
        if variation_selector(c) and (i == 0 or text[i - 1].isspace()
                                      or variation_selector(text[i - 1])):
            return "invisible"
    if most_marks(text) > max(MAX_MARKS, most_marks(english)):
        return "combining_marks"
    if not bidi_balanced(english, text):
        return "bidi"
    if formatting_codes(text) != formatting_codes(english):
        return "formatting_code"
    if text.count("\n") > 2 * english.count("\n"):
        return "line_break"
    if len(text) > max(LENGTH_FACTOR * len(english), len(english) + LENGTH_SLACK):
        return "length"
    if longest_run(text) > max(MAX_RUN, longest_run(english)):
        return "unbroken_run"
    return None


HOLE = re.compile(r"\{([A-Za-z0-9_]+)\}")


def holes(text: str) -> set[str]:
    """The `{name}` a launcher string fills in (`trf`)."""
    return set(HOLE.findall(text))


def entry_lines(body: str) -> list[str]:
    """The keys of the lines that hold one `"key": "value"`, in file order, duplicates
    included: the same count `lang.rs`'s test makes to catch a repeated key."""
    keys: list[str] = []
    for line in body.splitlines():
        if not line.startswith('  "'):
            continue
        try:
            value, _ = json.JSONDecoder().raw_decode(line.lstrip())
        except json.JSONDecodeError:
            continue
        if isinstance(value, str):
            keys.append(value)
    return keys


@dataclass
class LangFile:
    """One `lang/<code>.json` and whatever is wrong with its shape (its text is not checked here)."""

    path: Path
    entries: dict[str, str] = field(default_factory=dict)
    problem: str = ""

    @property
    def code(self) -> str:
        return self.path.stem

    def __post_init__(self):
        try:
            body = self.path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            self.problem = f"cannot read: {error}"
            return
        if body.startswith("\ufeff"):
            self.problem = "starts with a byte order mark"
            return
        try:
            data = json.loads(body)
        except json.JSONDecodeError as error:
            self.problem = f"not JSON: {error}"
            return
        if not isinstance(data, dict) or not all(
                isinstance(key, str) and isinstance(value, str) for key, value in data.items()):
            self.problem = "not a flat map of key to text"
            return
        self.entries = data
        if len(entry_lines(body)) != len(data):
            self.problem = ('a key repeats, or an entry is not one `"key": "value"` line with '
                            "two spaces of indentation")


def reviewed(path: Path) -> dict[str, str]:
    """The record of what a person wrote or checked in one lang file: key to the text as it
    was checked. No record means no entry is checked yet."""
    if not path.exists():
        return {}
    record = LangFile(path)
    if record.problem:
        raise ValueError(f"{shown(path)}: {record.problem}")
    return record.entries


def unreviewed(entries: dict[str, str], record: dict[str, str]) -> list[str]:
    """The entries no person has checked in their present text. An empty value is a gap, not a
    translation, so it is never one."""
    return sorted(key for key, value in entries.items() if value.strip() and record.get(key) != value)


def game_rows(directory: Path, codes: set[str],
              review: Path | None = None) -> tuple[LangFile, list[dict]]:
    """Every language in `directory` against `en_us.json`, most complete first. With `review`
    (a `lang_review/game` folder), each row also says which entries no person checked."""
    reference = LangFile(directory / REFERENCE)
    if reference.problem:
        raise ValueError(f"{shown(reference.path)}: {reference.problem}")
    if not reference.entries:
        raise ValueError(f"{shown(reference.path)}: no key at all")
    english = reference.entries
    rows = []
    for path in sorted(directory.glob("*.json")):
        if codes and path.stem not in codes:
            continue
        lang = LangFile(path)
        entries = lang.entries
        missing = sorted(set(english) - set(entries))
        extra = sorted(set(entries) - set(english))
        changed = []
        plain = unformattable()
        for key in sorted(set(entries) & set(english)):
            if not formats_like(key, english[key], entries[key], plain):
                changed.append({"key": key, "english": english[key], "value": entries[key]})
        same = [] if lang.code == "en_us" else sorted(
            key for key in set(entries) & set(english) if entries[key] == english[key])
        # A blank value is a gap, not a translation (see `unreviewed`), so it never covers its key.
        translated = sum(1 for key in english if entries.get(key, "").strip())
        rows.append({
            "code": lang.code,
            "path": shown(lang.path),
            "keys": len(entries),
            "coverage": translated / len(english),
            "missing": missing,
            "extra": extra,
            "placeholders": changed,
            "empty": sorted(key for key, value in entries.items() if not value.strip()),
            "same": same,
            "unreviewed": [] if lang.code == "en_us" else unreviewed(
                entries, reviewed(review / path.name) if review else {}),
            "problem": lang.problem,
        })
    rows.sort(key=lambda row: (-row["coverage"], row["code"]))
    return reference, rows


def crate_dirs(crates: Path) -> list[tuple[str, Path]]:
    """The crates that ship their own lang files: (crate name, its `assets/lang`), by name."""
    return sorted((path.parents[2].name, path.parent)
                  for path in crates.glob("*/assets/lang/" + REFERENCE))


def launcher_rows(directory: Path, codes: set[str], review: Path | None = None) -> list[dict]:
    """Every launcher language: the values still identical to their English key, the
    `{holes}` a value does not match it on, and with `review` (a `lang_review/launcher` folder)
    the entries no person checked. Its key set is the code's job (`i18n/tests.rs`)."""
    rows = []
    for path in sorted(directory.glob("*.json")):
        if codes and path.stem not in codes:
            continue
        lang = LangFile(path)
        same, changed = [], []
        for key, value in sorted(lang.entries.items()):
            if key == value:
                same.append(key)
                continue
            want, got = holes(key), holes(value)
            if want != got:
                changed.append({"key": key, "missing": sorted(want - got), "extra": sorted(got - want)})
        rows.append({
            "code": lang.code,
            "path": shown(lang.path),
            "keys": len(lang.entries),
            "same": same,
            "holes": changed,
            "empty": sorted(key for key, value in lang.entries.items() if not value.strip()),
            "unreviewed": unreviewed(lang.entries, reviewed(review / path.name) if review else {}),
            "problem": lang.problem,
        })
    return rows


def print_problems(rows: list[dict]) -> None:
    for row in rows:
        if row["problem"]:
            print(f"  ! {row['code']}: {row['problem']}")


def print_game(reference: LangFile, directory: Path, rows: list[dict], kind: str = "GAME") -> None:
    print(f"{kind}  {shown(directory)}  ({reference.code}: {len(reference.entries)} keys, the reference)")
    print()
    print(f"  {'code':<10}{'keys':>6}{'cover':>8}{'missing':>9}{'extra':>7}{'empty':>7}{'same':>6}"
          f"{'unreviewed':>12}")
    for row in rows:
        print(f"  {row['code']:<10}{row['keys']:>6}{100 * row['coverage']:>7.1f}%"
              f"{len(row['missing']):>9}{len(row['extra']):>7}{len(row['empty']):>7}"
              f"{len(row['same']):>6}{len(row['unreviewed']):>12}{'  *' if row['problem'] else ''}")
    print_problems(rows)


def print_launcher(directory: Path, rows: list[dict]) -> None:
    print(f"LAUNCHER  {shown(directory)}  (the English sentence is the key)")
    print()
    print(f"  {'code':<10}{'keys':>6}{'same':>7}{'holes':>7}{'empty':>7}{'unreviewed':>12}")
    for row in rows:
        print(f"  {row['code']:<10}{row['keys']:>6}{len(row['same']):>7}{len(row['holes']):>7}"
              f"{len(row['empty']):>7}{len(row['unreviewed']):>12}{'  *' if row['problem'] else ''}")
    print_problems(rows)


def print_groups(row: dict, groups: list[tuple[str, str]]) -> None:
    for field, title in groups:
        if row[field]:
            print(f"\n{row['code']}: {title} ({len(row[field])})")
            for key in row[field]:
                print(f"  {key}")


def print_game_keys(rows: list[dict]) -> None:
    for row in rows:
        print_groups(row, [("missing", "missing"), ("extra", "not in the reference"),
                           ("empty", "empty"), ("same", "identical to English")])
        for bad in row["placeholders"]:
            print(f"\n{row['code']}: {bad['key']} changes the placeholders of the English text, "
                  "or the game cannot format it")
            print(f"  english: {bad['english']}")
            print(f"  {row['code']}: {bad['value']}")


def print_launcher_keys(rows: list[dict]) -> None:
    for row in rows:
        print_groups(row, [("empty", "empty"), ("same", "identical to English")])
        for bad in row["holes"]:
            print(f"\n{row['code']}: {bad['key']} does not carry the key's holes")
            for side in ("missing", "extra"):
                if bad[side]:
                    print(f"  {side}: {', '.join(bad[side])}")


def print_game_summary(reference: LangFile, rows: list[dict], kind: str = "GAME") -> None:
    for row in rows:
        line = (f"{kind} {row['code']} {row['keys']}/{len(reference.entries)} keys, "
                f"missing {len(row['missing'])}, extra {len(row['extra'])}, "
                f"placeholders {len(row['placeholders'])}, empty {len(row['empty'])}, "
                f"same {len(row['same'])}, unreviewed {len(row['unreviewed'])}")
        print(line + (f", {row['problem']}" if row["problem"] else ""))


def print_launcher_summary(rows: list[dict]) -> None:
    for row in rows:
        line = (f"LAUNCHER {row['code']} {row['keys']} keys, same {len(row['same'])}, "
                f"holes {len(row['holes'])}, empty {len(row['empty'])}, "
                f"unreviewed {len(row['unreviewed'])}")
        print(line + (f", {row['problem']}" if row["problem"] else ""))


def print_unreviewed(kind: str, rows: list[dict], english: dict[str, str] | None) -> None:
    """The entries a reviewer reads, with the English beside them (a launcher key is its
    English)."""
    for row in rows:
        if not row["unreviewed"]:
            continue
        print(f"\n{kind} {row['code']}: {len(row['unreviewed'])} unreviewed ({row['path']})")
        text = LangFile(ROOT / row["path"]).entries  # ROOT / an absolute path is that path
        for key in row["unreviewed"]:
            print(f"  {key}")
            if english is not None:
                print(f"    en_us: {english.get(key, '')}")
            print(f"    {row['code']}: {text.get(key, '')}")


def mark_reviewed(lang: Path, record_path: Path, keys: list[str]) -> list[str]:
    """Records `keys` of `lang` as checked by a person, in their present text, and drops the
    record's keys the file no longer has. Returns the keys `lang` does not have."""
    entries = LangFile(lang).entries
    record = {key: text for key, text in reviewed(record_path).items() if key in entries}
    unknown = [key for key in keys if key not in entries]
    for key in keys:
        if key in entries:
            record[key] = entries[key]
    ordered = {key: record[key] for key in entries if key in record}
    if ordered:
        record_path.parent.mkdir(parents=True, exist_ok=True)
        record_path.write_text(json.dumps(ordered, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    elif record_path.exists():
        record_path.unlink()
    return unknown


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    which = parser.add_mutually_exclusive_group()
    which.add_argument("--game-only", action="store_true", help="only the game's assets/horizon/lang")
    which.add_argument("--launcher-only", action="store_true", help="only the launcher's lang")
    parser.add_argument("--game-dir", type=Path, default=GAME,
                        help="the game's lang folder (default: %(default)s)")
    parser.add_argument("--crates-dir", type=Path,
                        help="report every <dir>/<crate>/assets/lang too (default: the repository's crates, "
                             "unless --game-dir names another folder)")
    parser.add_argument("--launcher-dir", type=Path, default=LAUNCHER,
                        help="the launcher's lang folder (default: %(default)s)")
    parser.add_argument("--lang", nargs="+", metavar="CODE",
                        help="only these file stems (es_es and fr_fr for the game, es and fr for the launcher)")
    parser.add_argument("--summary", action="store_true", help="one line per file, for a pull request")
    parser.add_argument("--show-keys", action="store_true",
                        help="list the keys behind every count (--json already carries them)")
    parser.add_argument("--json", action="store_true", help="machine output")
    parser.add_argument("--fail-under", type=float, metavar="PERCENT",
                        help="exit 1 when a game language covers less than this")
    parser.add_argument("--review-dir", type=Path, default=REVIEW,
                        help="the record of checked entries, game/ and launcher/ (default: %(default)s)")
    parser.add_argument("--unreviewed", action="store_true",
                        help="list the entries no person checked yet, with their English")
    parser.add_argument("--mark-reviewed", nargs="*", metavar="KEY",
                        help="record these keys of the one --lang file as checked by a person")
    parser.add_argument("--keys-file", type=Path,
                        help="with --mark-reviewed: more keys, one per line (launcher keys are sentences)")
    args = parser.parse_args(argv)

    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    codes = set(args.lang or ())
    if args.crates_dir is None and args.game_dir == GAME:
        args.crates_dir = CRATES
    if args.mark_reviewed is not None:
        return mark_command(args, codes)
    if args.fail_under is not None and args.launcher_only:
        print("--fail-under measures the game's languages; --launcher-only has none", file=sys.stderr)
        return 2
    result: dict = {}
    game: list[dict] = []
    reference = None
    if not args.launcher_only:
        if not args.game_dir.is_dir():
            print(f"{args.game_dir}: no such folder", file=sys.stderr)
            return 2
        try:
            reference, game = game_rows(args.game_dir, codes, args.review_dir / "game")
        except ValueError as error:
            print(error, file=sys.stderr)
            return 2
        result["game"] = {"reference": reference.code, "keys": len(reference.entries), "files": game}
    # (crate name, its folder, its reference, its rows): the text crates ship.
    crates: list[tuple[str, Path, LangFile, list[dict]]] = []
    if not args.launcher_only and args.crates_dir is not None:
        for name, folder in crate_dirs(args.crates_dir):
            try:
                crates.append((name, folder, *game_rows(folder, codes, args.review_dir / name)))
            except ValueError as error:
                print(error, file=sys.stderr)
                return 2
        if crates:
            result["crates"] = {name: {"reference": ref.code, "keys": len(ref.entries), "files": rows}
                                for name, _, ref, rows in crates}
    launcher: list[dict] = []
    if not args.game_only:
        if not args.launcher_dir.is_dir():
            print(f"{args.launcher_dir}: no such folder", file=sys.stderr)
            return 2
        try:
            launcher = launcher_rows(args.launcher_dir, codes, args.review_dir / "launcher")
        except ValueError as error:
            print(error, file=sys.stderr)
            return 2
        result["launcher"] = {"files": launcher}
    if codes and not game and not launcher and not any(rows for *_, rows in crates):
        print(f"no lang file is named {' or '.join(sorted(codes))}", file=sys.stderr)
        return 2

    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    elif args.unreviewed:
        if reference:
            print_unreviewed("GAME", game, reference.entries)
        for name, _, ref, rows in crates:
            print_unreviewed(f"GAME {name}", rows, ref.entries)
        print_unreviewed("LAUNCHER", launcher, None)
    elif args.summary:
        if reference:
            print_game_summary(reference, game)
        for name, _, ref, rows in crates:
            print_game_summary(ref, rows, f"GAME {name}")
        print_launcher_summary(launcher)
    else:
        if reference:
            print_game(reference, args.game_dir, game)
        for name, folder, ref, rows in crates:
            print()
            print_game(ref, folder, rows, f"GAME {name}")
        if not args.game_only:
            if reference:
                print()
            print_launcher(args.launcher_dir, launcher)

    if args.show_keys and not args.json:
        # the keys behind the counts, after the lines that carry the counts; --json holds
        # them already, and printing more would break what a script reads.
        if reference:
            print_game_keys(game)
        for *_, rows in crates:
            print_game_keys(rows)
        if launcher:
            print_launcher_keys(launcher)

    if args.fail_under is not None:
        every = game + [row for *_, rows in crates for row in rows]
        under = [row for row in every if 100 * row["coverage"] + 1e-9 < args.fail_under]
        for row in under:
            print(f"{row['code']}: {100 * row['coverage']:.1f}% < {args.fail_under:g}%", file=sys.stderr)
        if under:
            return 1
    return 0


def mark_command(args, codes: set[str]) -> int:
    """`--mark-reviewed`: one language, named by `--lang`; --game-only or --launcher-only picks the
    folder when the code is in both. The game's text may be split between its own folder and the
    crates' (`--crates-dir`): each key is recorded for the folder that holds it."""
    if len(codes) != 1:
        print("--mark-reviewed needs exactly one --lang code", file=sys.stderr)
        return 2
    code = next(iter(codes))
    if code == "en_us":
        print("en_us is the English reference, not a translation", file=sys.stderr)
        return 2
    game = [("game", args.game_dir)]
    if args.crates_dir is not None:
        game += crate_dirs(args.crates_dir)
    sides = [[(kind, folder / f"{code}.json") for kind, folder in folders
              if (folder / f"{code}.json").exists()]
             for folders, skip in ((game, args.launcher_only), ([("launcher", args.launcher_dir)], args.game_only))
             if not skip]
    sides = [side for side in sides if side]
    if not sides:
        print(f"no lang file is named {code}", file=sys.stderr)
        return 2
    if len(sides) > 1:
        print(f"{code} is a game and a launcher file: add --game-only or --launcher-only", file=sys.stderr)
        return 2
    found = sides[0]
    keys = list(args.mark_reviewed)
    if args.keys_file:
        keys += [line for line in args.keys_file.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not keys:
        print("--mark-reviewed needs keys, on the command line or in --keys-file", file=sys.stderr)
        return 2
    # Each key goes to the first folder that has it; a key none has is reported against them all.
    left = list(keys)
    for kind, lang in found:
        entries = LangFile(lang).entries
        mine = [key for key in left if key in entries]
        left = [key for key in left if key not in entries]
        if not mine and len(found) > 1:
            continue
        try:
            mark_reviewed(lang, args.review_dir / kind / lang.name, mine)
        except ValueError as error:
            print(error, file=sys.stderr)
            return 2
        print(f"{len(mine)} entries of {shown(lang)} recorded as reviewed")
    for key in left:
        print(f"{' or '.join(shown(lang) for _, lang in found)} has no key {key!r}", file=sys.stderr)
    return 1 if left else 0


if __name__ == "__main__":
    raise SystemExit(main())
