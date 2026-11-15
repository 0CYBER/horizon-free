"""A `Result::unwrap` failure names its error's kind; no other payload text crosses, paths included."""
import contextlib
import io
import unittest

from tools.ci import ignored_tests as ci


class ErrorShape(unittest.TestCase):
    def render(self, value):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics("test load ... FAILED\n"
                                   "thread 'load' panicked at private.rs:1:1:\n"
                                   f"called `Result::unwrap()` on an `Err` value: {value}\n")
        return log.getvalue()

    def test_io_error_names_kind_and_os_error_but_no_path(self):
        log = self.render('Os { code: 2, kind: NotFound, message: "PRIVATE" } '
                          'reading /home/runner/work/horizon/.local/horizon-client/26.3/assets/minecraft/blockstates/stone.json')
        self.assertIn("panic reason: Result::unwrap on an error (kind Os, NotFound, os error 2 ENOENT); "
                      "rest of payload withheld (resource privacy)", log)
        self.assertNotIn("stone.json", log)
        self.assertNotIn("PRIVATE", log)
        self.assertNotIn("/home/runner", log)

    def test_message_without_a_kind_or_path_stays_fully_withheld(self):
        log = self.render('"PRIVATE text with a value 42"')
        self.assertIn("panic reason: Result::unwrap on an error; payload withheld (resource privacy)", log)
        self.assertNotIn("PRIVATE", log)

    def test_a_quoted_string_payload_with_kind_like_text_stays_fully_withheld(self):
        log = self.render('"failed to parse kind: PrivateResourceName (os error 2)"')
        self.assertIn("panic reason: Result::unwrap on an error; payload withheld (resource privacy)", log)
        self.assertNotIn("PrivateResourceName", log)
        self.assertNotIn("os error", log)

    def test_fields_outside_the_io_error_shape_or_kind_list_are_not_named(self):
        for value in ('failed to parse kind: PrivateResourceName (os error 2)',
                      'PrivateVariant { kind: NotFound, code: 2 }'):
            with self.subTest(value=value):
                self.assertIsNone(ci.unwrap_error_shape(value))
        # The quoted message's text is not read; a kind outside std's ErrorKind list is not named.
        self.assertEqual(ci.unwrap_error_shape('Custom { kind: Other, error: "kind: PrivateResourceName code: 7" }'),
                         "kind Custom, Other")
        self.assertEqual(ci.unwrap_error_shape("Kind(PrivateResourceName)"), "kind Kind")


if __name__ == "__main__":
    unittest.main()
