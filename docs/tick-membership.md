# Reuse distant-terrain tick membership storage

DH 3.3.4's tick holder clears and repopulates a private `HashSet` of nodes every
tick. Clearing retains the table but discards its entry objects, so handling the
same terrain again creates new entries. A local allocation profile identified
these entries in the `DH-ClientTickTimer` tree traversal.

The optional hook uses the game's existing fastutil `ObjectOpenHashSet` for only
this private membership test. Its reusable array stores the references directly.
The original branches, published action sets/lists, action iteration order,
tree traversal, node identity, detail levels and render settings stay upstream.
The replacement clears at the original tick boundary, releasing member references
while retaining table capacity. It adds no node cache, global pool or renderer.

Use `tick-dedup` for an isolated comparison with `m2-balanced`; their visual,
simulation, worker and other controller settings match. From version 0.13 the
combined `foreground-first` and `foreground-vulkan` experiments include this
hook too. Other profiles explicitly disable it. Set `enabled=false` in
`config/horizon-tick-dedup.properties` and restart to disable it. The hook only
applies to pinned DH 3.3.4. Reports record actual `tick_membership_dedup` state;
the comparison tools disclose differing state as an experiment variable.

`python tools/test_tick_dedup.py` runs the publisher's actual holder inside two
isolated Fabric clients. Both check 6,000 deterministic randomized transitions,
including duplicate/null membership, first-action precedence, forced disable,
clear boundaries and published collection order. A warmed allocation loop uses
512 preallocated nodes, duplicate visits and the reused delete-children action
list for 20,000 ticks. It measures only the calling thread's allocated bytes,
CPU and elapsed time, keeping membership allocation separate from node creation.
The fixture JAR and full game logs remain private; the sanitized JSON result
contains no game code, credentials or launch arguments.

The local Linux check allocated 245,760,208 bytes upstream versus 208 with the
hook. Its single CPU observation was 565 ms versus 381 ms. This establishes the
entry-allocation mechanism in that isolated loop; it does not establish a
statistical speedup, game FPS gain, whole-game RAM reduction, completed terrain
or performance on an M2 GPU. Actual game profiles and repeated Mac comparisons
are required for those conclusions. Table capacity remains proportional to the
largest tick membership, as the upstream hash table already did.
