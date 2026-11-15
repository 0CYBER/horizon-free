"""command_exec groups, one file each (#16833).

A module here defines `GROUP`, the group's name, and `cases(cmd, setup)`, which returns its ops
built with the corpus's `cmd` and `setup`. `command_exec_corpus.discovered_groups()` finds the
modules by file name, so a new group needs no line anywhere else. Files starting with `_` are
skipped. Give each group an area of its own and clean it up, as the groups in
`tools/command_exec_corpus.py` do.
"""
