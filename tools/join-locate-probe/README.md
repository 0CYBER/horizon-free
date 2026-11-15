# Production-source join lookup proof

The probe imports the actual client `join_seq.rs`, with only the dependencies those files reach
(`horizon_core`, the mod API's `mod_api` vocabulary, the attachment runtime and `log`);
it does not implement lookup or attachment lifetime behavior a second time.

```sh
cargo test --manifest-path tools/join-locate-probe/Cargo.toml --lib join_seq::
python3 tools/join-locate-probe/mutate.py
# With prepared resources and isolated game homes:
python3 tools/join-locate-probe/dispatch_mutate.py
```

The mutation script changes unique production-source anchors, never tests; requires
an actual failed test, not a compiler error; and restores the source in `finally`.
Run it without concurrent builds of this source. Debug assertions must be enabled
for the bound-update overwrite fixture (as in the commands above).

| Production site | Fault | Killing test |
|---|---|---|
| locate Ok arm | wrong hit slot | locate_finds_ordered_reordered_and_temporarily_absent_owners |
| locate Ok arm | linear first match instead of binary duplicate selection | locate_keeps_binary_hit_selection_for_duplicate_joins |
| store_back candidate | skip direct index check and always search | store_back_uses_its_supplied_slot_without_searching |
| locate Err arm | omit fallback | locate_finds_ordered_reordered_and_temporarily_absent_owners |
| locate fallback predicate | inverted key comparison | locate_finds_ordered_reordered_and_temporarily_absent_owners |
| store_back join | replace clone's join with a new number | store_back_preserves_join_and_registration_after_reordering |
| store_back candidate filter | accept another owner's slot | store_back_preserves_join_and_registration_after_reordering |
| store_back relocation | omit lookup | store_back_preserves_join_and_registration_after_reordering |
| store_back absent return | accept removed owner | store_back_preserves_join_and_registration_after_reordering |
| store_back lifetime swap | drop old registration | store_back_preserves_join_and_registration_after_reordering |
| checked_edit assertion | bypass registration check | bound_update_rejects_whole_entity_overwrite |
| ordered_tick TNT caller | use slot zero instead of located slot | dispatch_ticks_both_owners_of_each_located_kind |
| ordered_tick falling caller | use slot zero instead of located slot | dispatch_ticks_both_owners_of_each_located_kind |
| ordered_tick orb caller | use slot zero instead of located slot | dispatch_ticks_both_owners_of_each_located_kind |

This is a semantic fault inventory, not exhaustive mutation of equivalent expressions.
The Game fixture `ordered_tick::tests::dispatch_ticks_both_owners_of_each_located_kind`
summons two TNTs, two falling blocks and two orbs, then runs the real ordered-tick dispatcher and
asserts both owners advance exactly once. `dispatch_mutate.py` separately forces slot
zero in each of the three dispatch arms; all three faults must fail this fixture.
It needs prepared resources and is ignored by the ordinary workspace test run.
The old ordered-tick lookup test moves into join_seq with expanded take/restore coverage.
The actual dimension-transfer path's fresh-destination-join regression is owned by
arismoko/horizon#1771 (`transfer_arrival_rebinds_only_bound_owners_with_the_same_id`).

No per-removal metadata, tombstones, compatibility path or new residency protocol.
The table stores join numbers, not list positions; ordinary structural operations
retain their existing Vec costs. Out-of-order and absent joins retain the old linear
fallback. The cost and batching experiments that rejected the position index are
preserved on the superseded arismoko/horizon#1772 and arismoko/horizon#1776 branches, not copied into production.
