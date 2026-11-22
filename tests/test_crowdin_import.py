import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from tools import crowdin_import, lang_coverage

ENGLISH = {
    "horizon.a": "Apple",
    "horizon.b": "Banana %s",
    "horizon.c": "Cherry",
}


def write(folder: Path, name: str, entries: dict[str, str]) -> Path:
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / name
    path.write_text(json.dumps(entries, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return path


def run(*arguments: str) -> tuple[int, str, str]:
    """The tool's exit code and its two streams."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            code = crowdin_import.main([str(argument) for argument in arguments])
        except SystemExit as exit_code:
            code = exit_code.code
    return code, out.getvalue(), err.getvalue()


class TableTests(unittest.TestCase):
    def test_every_lang_file_has_a_crowdin_language_and_the_other_way_round(self):
        self.assertEqual(crowdin_import.check(), [],
                         "a lang file without a Crowdin language, or a Crowdin language without "
                         "a lang file, is an error: docs/translations.md")
        codes = {path.stem for path in crowdin_import.LANG.glob("*.json")} - {"en_us"}
        self.assertEqual(codes, set(crowdin_import.LOCALES) | set(crowdin_import.NOT_IN_CROWDIN))

    def test_a_lang_file_no_one_has_placed_complains(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            write(folder, "en_us.json", ENGLISH)
            for code in crowdin_import.LOCALES | crowdin_import.NOT_IN_CROWDIN:
                write(folder, f"{code}.json", {})
            write(folder, "xx_yy.json", {"horizon.a": "Manzana"})
            problems = crowdin_import.check(folder)
        self.assertEqual(len(problems), 1)
        self.assertIn("xx_yy", problems[0])

    def test_crowdin_writes_a_locale_with_underscores(self):
        table = crowdin_import.index()
        for locale, code in [("sv_SE", "sv_se"), ("pt_BR", "pt_br"), ("zh_CN", "zh_cn")]:
            with self.subTest(locale=locale):
                self.assertEqual(crowdin_import.language(locale, table), code)

    def test_a_locale_with_a_region_the_project_does_not_name_resolves_by_its_subtag(self):
        table = crowdin_import.index()
        for locale, code in [("uk_UA", "uk_ua"), ("de_DE", "de_de"), ("ru_RU", "ru_ru")]:
            with self.subTest(locale=locale):
                self.assertEqual(crowdin_import.language(locale, table), code)

    def test_a_language_without_its_region_resolves_by_its_subtag(self):
        table = crowdin_import.index()
        self.assertEqual(crowdin_import.language("fr", table), "fr_fr")
        self.assertEqual(crowdin_import.language("de", table), "de_de")

    def test_a_subtag_two_codes_claim_is_not_resolved_by_the_subtag(self):
        table = crowdin_import.index()
        self.assertNotIn("pt", table)
        self.assertEqual(table["sr"], "sr_sp")  # Serbian (Cyrillic) names the locale `sr` outright
        self.assertEqual(crowdin_import.language("sr-CS", table), "sr_cs")
        self.assertEqual(crowdin_import.language("sr", table), "sr_sp")


class MergeTests(unittest.TestCase):
    def test_the_file_keeps_its_order_and_gains_new_keys_in_english_order(self):
        existing = {"horizon.c": "Cereza", "horizon.a": "Manzana"}
        incoming = {"horizon.a": "Manzana", "horizon.c": "Cereza", "horizon.b": "Plátano %s"}
        merged, added, changed = crowdin_import.merge(existing, incoming, list(ENGLISH))
        self.assertEqual(list(merged), ["horizon.c", "horizon.a", "horizon.b"])
        self.assertEqual(added, ["horizon.b"])
        self.assertEqual(changed, 0)

    def test_a_key_english_dropped_keeps_its_text_and_a_changed_one_counts(self):
        existing = {"horizon.a": "Manzana", "horizon.gone": "Vieja"}
        merged, added, changed = crowdin_import.merge(
            existing, {"horizon.a": "Pera"}, list(ENGLISH))
        self.assertEqual(merged["horizon.gone"], "Vieja")
        self.assertEqual(merged["horizon.a"], "Pera")
        self.assertEqual(changed, 1)
        self.assertEqual(added, [])


class SyncTests(unittest.TestCase):
    def fixture(self, tmp: str) -> tuple[Path, Path, Path, Path]:
        root = Path(tmp)
        lang, review = root / "lang", root / "review"
        write(lang, "en_us.json", ENGLISH)
        write(lang, "es_es.json", {"horizon.a": "Manzana", "horizon.b": "Plátano %s",
                                   "horizon.c": "Cereza"})
        return root, lang, review, root / "export"

    def test_an_approval_is_recorded_with_the_text_the_file_has(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(export, "es-ES.json", {"horizon.a": "Manzana", "horizon.b": "Plátano %s",
                                         "horizon.c": "Picota"})
            approved = write(root / "approved", "es-ES.json", {"horizon.c": "Picota"})
            code, out, _ = run("--translations", export, "--approved", approved.parent,
                               "--lang", lang, "--review", review)
            self.assertEqual(code, 0, out)
            self.assertEqual(json.loads((lang / "es_es.json").read_text(encoding="utf-8"))["horizon.c"],
                             "Picota")
            self.assertEqual(json.loads((review / "es_es.json").read_text(encoding="utf-8")),
                             {"horizon.c": "Picota"})
            self.assertIn("1 recorded as reviewed", out)

    def test_a_later_edit_without_a_proofread_takes_the_mark_off(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(review, "es_es.json", {"horizon.c": "Picota"})
            write(export, "es-ES.json", {"horizon.c": "Guinda"})
            (root / "approved").mkdir()
            run("--translations", export, "--approved", root / "approved", "--lang", lang,
                "--review", review)
            rows = lang_coverage.game_rows(lang, set(), review)[1]
            row = next(row for row in rows if row["code"] == "es_es")
            self.assertIn("horizon.c", row["unreviewed"])

    def test_a_translation_that_changes_a_placeholder_is_dropped_and_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(export, "es-ES.json", {"horizon.b": "Plátano %d"})
            code, _, err = run("--translations", export, "--approved", root / "approved",
                               "--lang", lang, "--review", review)
            self.assertEqual(code, 0)
            entries = json.loads((lang / "es_es.json").read_text(encoding="utf-8"))
            self.assertEqual(entries["horizon.b"], "Plátano %s")
            self.assertIn("horizon.b", err)

    def test_a_language_the_table_does_not_name_fails_the_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(export, "es-ES.json", {"horizon.a": "Manzana"})
            write(export, "klingon.json", {"horizon.a": "…"})
            before = (lang / "es_es.json").read_text(encoding="utf-8")
            code, _, err = run("--translations", export, "--approved", root / "approved",
                               "--lang", lang, "--review", review)
            self.assertEqual(code, 1)
            self.assertIn("klingon", err)
            self.assertEqual((lang / "es_es.json").read_text(encoding="utf-8"), before)

    def test_a_dry_run_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(export, "es-ES.json", {"horizon.c": "Picota"})
            before = (lang / "es_es.json").read_text(encoding="utf-8")
            code, out, _ = run("--dry-run", "--translations", export, "--approved",
                               root / "approved", "--lang", lang, "--review", review)
            self.assertEqual(code, 0, out)
            self.assertEqual((lang / "es_es.json").read_text(encoding="utf-8"), before)
            self.assertFalse((review / "es_es.json").exists())

    def test_the_written_file_is_one_entry_per_line_with_the_newline_it_had(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            (lang / "es_es.json").write_bytes(
                '{\r\n  "horizon.a": "Manzana"\r\n}\r\n'.encode("utf-8"))
            write(export, "es-ES.json", {"horizon.a": "Manzana"})
            run("--translations", export, "--approved", root / "approved", "--lang", lang,
                "--review", review)
            raw = (lang / "es_es.json").read_bytes()
            self.assertIn(b"\r\n", raw)
            self.assertEqual(lang_coverage.LangFile(lang / "es_es.json").problem, "")
            entries = json.loads(raw.decode("utf-8"))
            self.assertEqual(list(entries), ["horizon.a"])

    def test_a_string_crowdin_left_untranslated_does_not_replace_a_translation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            # Crowdin's default export writes the English sentence into an untranslated string.
            write(export, "es-ES.json", {"horizon.a": "Manzana", "horizon.b": "Banana %s"})
            code, _, err = run("--translations", export, "--approved", root / "approved",
                               "--lang", lang, "--review", review)
            self.assertEqual(code, 0, err)
            entries = json.loads((lang / "es_es.json").read_text(encoding="utf-8"))
            self.assertEqual(entries["horizon.b"], "Plátano %s")
            self.assertIn("horizon.b", err)

    def test_english_is_kept_where_the_language_really_writes_the_english_word(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            (lang / "es_es.json").write_bytes(
                json.dumps({"horizon.a": "Apple"}, ensure_ascii=False, indent=2).encode("utf-8"))
            write(export, "es-ES.json", {"horizon.a": "Apple"})
            run("--translations", export, "--approved", root / "approved", "--lang", lang,
                "--review", review)
            entries = json.loads((lang / "es_es.json").read_text(encoding="utf-8"))
            self.assertEqual(entries["horizon.a"], "Apple")

    def test_a_withdrawn_approval_takes_its_mark_back(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            approved = root / "approved"
            write(export, "es-ES.json", {"horizon.c": "Picota"})
            write(approved, "es-ES.json", {"horizon.c": "Picota"})
            run("--translations", export, "--approved", approved, "--lang", lang, "--review", review)
            record = json.loads((review / "es_es.json").read_text(encoding="utf-8"))
            self.assertEqual(record, {"horizon.c": "Picota"})

            # the proofread is withdrawn in Crowdin, the text stays what it was
            for path in approved.glob("*.json"):
                path.unlink()
            run("--translations", export, "--approved", approved, "--lang", lang, "--review", review)
            self.assertFalse((review / "es_es.json").exists())
            rows = lang_coverage.game_rows(lang, set(), review)[1]
            row = next(row for row in rows if row["code"] == "es_es")
            self.assertIn("horizon.c", row["unreviewed"])

    def test_a_persons_own_entry_survives_the_sync(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(review, "es_es.json", {"horizon.a": "Manzana"})  # marked by hand, from a report
            write(export, "es-ES.json", {"horizon.c": "Picota"})
            run("--translations", export, "--approved", root / "approved", "--lang", lang,
                "--review", review)
            record = json.loads((review / "es_es.json").read_text(encoding="utf-8"))
            self.assertEqual(record, {"horizon.a": "Manzana"})

    def test_staging_writes_the_names_crowdin_reads_and_leaves_the_hand_kept_languages(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, lang, review, export = self.fixture(tmp)
            write(lang, "lmo.json", {"horizon.a": "Poma"})
            folder = root / "stage"
            rows, hand_kept = crowdin_import.stage(lang, folder)
            self.assertEqual({row["code"] for row in rows}, {"es_es"})
            self.assertEqual(hand_kept, ["lmo"])
            self.assertTrue((folder / "es_ES.json").exists())
            self.assertEqual(list(json.loads((folder / "es_ES.json").read_text(encoding="utf-8"))),
                             list(ENGLISH))


if __name__ == "__main__":
    unittest.main()
