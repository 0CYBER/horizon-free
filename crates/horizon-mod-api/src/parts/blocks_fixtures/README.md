# Part fixture fragments

Add a Rust file for each new method group's fixture, recording helpers and tests.
The build script discovers `.rs` files here and in `../shared_fixtures`, sorted by
path, and includes them in the original `tests` module. Do not add module entries
or edit the shared include list. Fixture paths and test names do not gain an
extra module segment. Items share the original imports and privacy boundary.

The one-shot extraction is recorded in `tools/split_part_fixtures.py`.
