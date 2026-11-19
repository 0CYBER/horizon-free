# Moving launcher data

In the launcher's Settings, open About → Data folder → Move. Choose an empty
folder with enough free space. The launcher closes while its offline worker copies
and verifies launcher data, worlds and build data, then reopens at the new location.
The originals remain until you verify the move and remove them yourself.

On Windows, the selected folder may contain spaces or non-English characters.
Build tools still need a real ASCII path without whitespace: a short name or junction
can be resolved back to the incompatible name by the compiler. When needed, the
worker creates a fresh `horizon-build-<unique>/build` directory under the nearest
writable compatible ancestor on the destination drive. If none is available there,
it tries compatible ancestors of the old build folder. Settings shows the active
build-data path; do not remove it when cleaning up the old copies.

The build directory is copied, not reset. Its files, including the toolchain and
Cargo cache, are verified before the location pointer changes. A subsequent move
to a compatible path brings that directory back under the chosen destination.
If the old build root is also inside (or equal to) the old launcher root, the launcher
copy retains those files too; allow space for both copies when splitting them.

A failed copy, preparation or pointer commit leaves the old location active and
preserves the original data. The error names both possible partial-copy locations.
Retry with an empty destination; each attempt reserves its own build directory and
never overwrites an earlier attempt. Reconnect a missing drive before restarting:
the existing missing-root recovery applies to the separate build folder as well.
