"""command_exec groups in files of their own are found by file name (#16833).

tools/command_exec_corpus.py was the one place every new group had to add a function and a list
line, so two pull requests that added a group conflicted. A module under tools/command_exec_groups/
needs no shared line.
"""
from pathlib import Path
import tempfile
import unittest

from tools import command_exec_corpus as corpus

MODULE = """GROUP = {name!r}


def cases(cmd, setup):
    return [cmd({command!r})] + setup('/say done')
"""


class GroupFileTests(unittest.TestCase):
    def test_modules_are_found_sorted_by_file_name_and_given_cmd_and_setup(self):
        with tempfile.TemporaryDirectory() as folder:
            for file, name in (('b_second.py', 'second'), ('a_first.py', 'first'), ('_skipped.py', 'skipped')):
                Path(folder, file).write_text(MODULE.format(name=name, command=f'/say {name}'), encoding='utf-8')
            found = corpus.discovered_groups(folder)
        self.assertEqual(['first', 'second'], [name for name, _ in found])
        self.assertEqual([corpus.cmd('/say first'), *corpus.setup('/say done')], found[0][1])

    def test_discovered_groups_go_before_the_last_group(self):
        merged = corpus.with_discovered([('a', []), ('last', [])], [('found', [])])
        self.assertEqual(['a', 'found', 'last'], [name for name, _ in merged])

    def test_a_name_used_twice_is_an_error(self):
        with self.assertRaisesRegex(ValueError, 'used twice: a'):
            corpus.with_discovered([('a', []), ('last', [])], [('a', [])])

    def test_the_corpus_includes_the_group_files(self):
        names = [name for name, _ in corpus.groups()]
        found = [name for name, _ in corpus.discovered_groups()]
        self.assertIn('thrown item kept', found)
        self.assertEqual(found, names[-1 - len(found):-1])
        self.assertEqual(len(names), len(set(names)))


if __name__ == '__main__':
    unittest.main()
