"""Symlinks in tests: Windows lets an account make them only with Developer Mode on or as an administrator."""
import unittest

ERROR_PRIVILEGE_NOT_HELD = 1314


def symlink_or_skip(link, target, target_is_directory=False):
    """Make `link` point at `target`, or skip the test where Windows refuses this account a symlink. Any other
    error, on any system, is the test's to see."""
    try:
        link.symlink_to(target, target_is_directory=target_is_directory)
    except OSError as error:
        if getattr(error, "winerror", None) != ERROR_PRIVILEGE_NOT_HELD:
            raise
        raise unittest.SkipTest("Windows makes symlinks only with Developer Mode on or as an administrator") from error
