# Block-entity item overlay performance

The accepted implementation uses a thin entry that tests the plain pending-overlay field. Its original body remains a separate function, without a new inlining attribute. Each changed read has one cold, non-inlined scoped implementation. The tick enters with copy-only metadata and creates the item overlay only on first submission. No atomics, per-tick global or thread-local lookup, or scan of mods is added to these paths.

## Measurements and accepted cost

These are `fast` profile measurements on one box, without concurrent Cargo or rustc processes in that box. Activity outside the box was not controlled. Each result is the median of 3 sequential runs. Every run records 240 samples after 20 warmups. Ticks use the existing `thousand_live_block_entity_ticks` benchmark with its trivial attached native mod; reads use `unscoped_item_entity_reads` against a real Game outside any scope.

| Variant | Nanoseconds per 1,000 ticks | Nanoseconds per 10,000 reads |
| --- | ---: | ---: |
| Unchanged runtime | 102,653 | 216,707 |
| Initial scope entry only | 112,721 | 261,361 |
| Initial entry and split writeback | 123,191 | 261,370 |
| Initial full overlay | 139,692 | 261,801 |
| Lazy no-item path | 110,467 | 252,633 |
| Lazy scope and cold proposal lookup | 102,894 | 251,904 |
| Thin-entry checkpoint | 103,273 | 258,515 |
| Clean rebuilt baseline, final verification | 103,293 | 215,562 |
| Final acceptance and cleanup checks | 107,061 | 258,715 |

At the thin-entry checkpoint, ticks were 0.60% above the initial baseline and reads cost 25.85 ns each versus 21.67 ns unchanged: about 4.2 ns extra, or 19.29%. That read-path trade-off was accepted explicitly, not described as noise. Subsequent correctness checks close abandoned explicit transactions and detach the active key before publication callbacks; no further performance optimization was attempted.

Final verification against a separately rebuilt clean baseline records a tick cost of 107.06 ns versus 103.29 ns each, or 3.65% overhead. Tick run medians are 106,860–107,091 ns per 1,000, versus baseline 103,022–103,333 ns. Unscoped reads cost 25.87 ns versus 21.56 ns each: 4.32 ns extra, or 20.02%. Read run medians are 258,495–264,666 ns per 10,000, versus baseline 214,981–215,732 ns. These costs are reported plainly; the independent reviewer reruns both benchmarks.

The final baseline is revision `686c686e75` with only the identical read-benchmark fixture added. It builds into a separate target directory: a shared-target executable used earlier was overwritten by later builds, detected by its added `unscoped_*` symbols, and excluded from the comparison.

A later rebuilt unchanged baseline remained faster: tick median 102,703 ns and read median 214,001 ns. The difference is not explained by the initial baseline simply drifting up to the changed result. Earlier debug-profile measurements overstated absolute tick costs and are not the decision numbers.

## Compiler findings

Direct `nm` and `objdump` inspection of the optimized executable showed that the initial tick eagerly saved and initialized 104 bytes of Drop-bearing scope, including empty collection bookkeeping. Moving that state behind lazy first-write allocation removed the tick regression.

A borrowed-only cold proposal lookup was insufficient for reads: the compiler retained duplicate stack-clone sequences in the hot reader, increased its code from 335 to 507 bytes, increased saved registers from 5 to 6 and the stack frame from 32 to 40 bytes, and delayed dropped-list metadata loads until after the resolver call. The final extraction puts the whole scoped stack read in its cold function and restores the original unscoped body's 5-register, 32-byte frame and preload scheduling.

The extracted resolver is the same 392-byte instruction sequence as unchanged, except Game field offsets and linkage relocations. The extracted stack body is 332 bytes versus 335 unchanged: LLVM drops the unused internal return-pointer move `mov %rbx,%rax`. At the thin-entry checkpoint, Game's new state shifted relevant field offsets by 48 bytes even when declared at the struct's end; the final key layout no longer shifts the fields in these two bodies. The final resolver has no instruction difference after normalizing linked branch targets and relocations. The stack body differs only by that omitted internal return-pointer move. Layout, relocation and unused internal-return differences were accepted as unavoidable consequences, not changes to the original body. Literal linked bytes are not claimed identical. The instruction comparisons are [the stack-body diff](block-entity-tick-items-stack.diff) and [the resolver-body diff](block-entity-tick-items-resolver.diff).

The lock increment visible in both stack bodies belongs to the existing cloned ItemStack's Arc. It is not a new overlay or failure-counter atomic operation.
