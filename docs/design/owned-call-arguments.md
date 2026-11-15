# Owned arguments at native source call sites

Design proposal for arismoko/horizon#2871, port mixin row 2; no implementation is authorized by this note.
The confirmed source call is `Game::dispense_item(pos: Pos, facing: Direction,
stack: ItemStack) -> Result<Option<ItemStack>>`, inside `Game::dispense_from`.
The SDK refuses it with `argument record requires a borrowing or non-Copy field`.
`ItemStack` is owned and non-Copy: the repeatable continuation used for scalar records
cannot call the original twice without inventing another stack.

## Ownership contract

This proposal applies only to native `[[native.mixin]]` source transformations, not API injection points. Argument expressions are
evaluated once, in their original order, and their values **move** into one record.
Neither lowering nor dispatch clones a stack, implicitly or explicitly.

| Injection contract | What it receives | Fate of the owned value |
| --- | --- | --- |
| Observing before-call listener | `&Args`, hence `&ItemStack` | The unchanged record moves to the next listener, then the original consumes it. |
| Mutating before / argument modifier | `&mut Args`, hence `&mut ItemStack` | The next listener and original receive the modified value. Replacement drops the previous value unless the callback explicitly moves it elsewhere. |
| Wrap | Ownership of `Args`, plus a move-only original capability | It may forward the record exactly once, or return its own output without forwarding. It can inspect or mutate fields before forwarding. |
| Cancel / wrap without original | The same owned record | Unconsumed fields drop when the record leaves scope. For non-unit output the wrapper must supply a correctly typed result. |

An observing listener is not given ownership of the stack. A modifier may replace it
through `&mut T` using ordinary Rust operations; any explicit mod-side ownership transfer
is the mod's responsibility, not an engine-created copy. After forwarding, neither the
wrapper nor the engine retains the consumed record. Result-only modification runs after
consumption and receives only the owned result, not the old arguments. Unwinding follows
Rust drop rules; there is no manual duplicate cleanup.

Illustrative lowering, not a proposed public symbol spelling:

```rust,ignore
struct Args { pos: Pos, facing: Direction, stack: ItemStack }
let mut args = Args { pos, facing, stack }; // moves stack, never clones
observe(&args.stack);
modify_argument(&mut args.stack);
let original = |args: Args| {
    game.dispense_item(args.pos, args.facing, args.stack) // consumes record
};
let result = wrap(args, original); // original is FnOnce, not repeatable FnMut
// A cancelling wrap returns its own result and lets args drop instead.
```

This requires a distinct **single-use continuation contract**, not a compatibility shim
for repeatable wraps. Mixin lowering must distinguish it from a repeatable
continuation type. A generated move-only `Original` capability can consume `self` and `Args`
when invoking a private stack-borrowed dispatch continuation; it must not be Clone, leak a
repeatable callable, allocate a box per call, or offer a second-call panic/fallback.
Nested wrappers each receive one forwarding capability. Repeated-original wraps remain
available only where every argument supports the existing repeatable contract.

## Borrowed fields and refusals

Use **one record containing both borrowed and owned fields**. The lifetime-bearing record
from [the borrowing-record work](https://github.com/arismoko/horizon/pull/2872) can move
while its shared references remain call-scoped. Moving the record into original consumes
its owned fields and copies its shared-reference fields; cancellation drops owned fields,
not referents. Engine-type lifetimes remain separate from the record borrow lifetime.
A record with any owned non-Copy field always selects the single-use contract.

Mutable-reference arguments still refuse: preserving exclusive reborrows is not covered.
API injection points retain the existing refusal
rather than changing a prebuilt interface or introducing an API source adapter. Unresolved
or unrepresentable types retain the existing named record refusal. Borrowed results and
alias-dependent receiver arguments retain their existing signature/alias refusals. Before
any write, canonical identities, function hashes, ordinal/context freshness and eligibility
must pass; refusal leaves source byte for byte unchanged. Building this proposal waits for
root's decision after independent review.
