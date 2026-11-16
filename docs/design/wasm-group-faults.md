# What happens to a wasm dependency group after a trap or an interruption

Design note for arismoko/horizon#2766 (interruption counted by work, not by wall clock) and arismoko/horizon#2761 (one failing mod shouldn't
disable its whole group). Both come down to one question: when a call into a group fails, what is still safe to run?
This note answers that question. It changes no code. The facts below are from `crates/horizon-mod-wasm` at main
fbf4f06490, except where a line says a stage has landed.

## Decisions

1. **Classify each failure by whether guest code was cut off midway.** A *refusal* happens when guest code returned
   normally or never ran: the guest's error reply, a reply the host can't decode, the depth limit, a busy group.
   A *fault* happens when guest code stopped midway: a trap (including a guest panic, which is a trap under
   `panic_abort`), an import error that traps the guest, or an interruption.
2. **A refusal no longer stops the group.** The call answers as unavailable, the same as today, and logs once per
   listener with the reason. The group keeps running. Today every refusal except a busy group stops the group.
3. **A fault stops the whole group, and the group is never resumed.** Group memory, the allocator in the std member,
   library statics and the guest's per-depth call stacks may be half-updated. A per-member stop flag over that memory
   would not make continuing safe, so none is added.
4. **Recovery means rebuilding the group, not resuming it.** At a safe point a new group is instantiated through the
   existing load path, with the faulting member treated as refused at load. Its dependents go with it, by the rule
   load already applies to a required dependency that is missing. Each library is still one copy in one memory per
   group, and no second mechanism is added. Until a rebuild exists, the stop lasts until the next start, as today.
5. **The runaway budget is fuel: deterministic guest work.** Wasmtime fuel is consumed only by guest instructions,
   from one tank per store. So time spent in host imports, time the thread is descheduled and machine load never
   count, and nested calls draw on the outer call's tank. The same call uses the same fuel on any machine. Epoch
   interruption and its timer thread are removed. Fuel's cost is measured before it lands (§1). If that cost is too
   high, the fallback is a wall-clock heuristic, documented as best effort and not deterministic, not a mix of the
   two.
8. **A fault aborts the transaction it happened in.** When the faulted call runs inside a transaction (decide, fire,
   apply, or an item transaction), that transaction is aborted, never committed. Writes made through imports outside
   any transaction stay applied, as they do today. This is decided before recovery is built, and the rebuild (4)
   waits until it is implemented and tested.
6. **A fault names three things:** the member whose hook was entered, the member whose code was running at the trap
   (the innermost wasm frame), and the call path through the host. It goes to the log, to the Mods page and to
   operators, so it is never silent.
7. **Stopped members are inert and say so.** Their listeners, Parts and providers are skipped, as `is_stopped`
   already does. A block entity whose Part belongs to a stopped group keeps its saved data unchanged, so it resumes
   when the group runs again. The Mods page shows the group as stopped with its fault.

## Where things stand

- **One store per group.** `WasmMod::instantiate` (`runtime.rs:439-531`) builds one `Store`, links every member into
  one memory through `Group::link_with`, and gives each member's `WasmMod` the same `Arc<Running>`. `Running` holds
  the store and a single `poisoned` flag. A group is created once, at startup (`entry.rs:98` → `load_installed`); a
  world reload does not rebuild it.
- **Every failure stops the group.** `call` returns `None` on an already-stopped group (`:551`). It stops the group
  when the depth passes 64 (`:605`) and on any `Err` from the call body (`:664`). That `Err` covers traps,
  interruptions, import errors, the guest's `u32::MAX` error reply (`invoke`), and replies the host can't decode or
  that have trailing bytes. Only a busy group (`Refusal::Busy`) returns `None` without stopping it. Nothing clears
  the flag.
- **Stage 1 has landed: refusals no longer stop the group.** `invoke` now classifies each `Err` as
  `CallError::Refused` (guest code returned normally or never ran: the guest's error reply, a reply the host can't
  decode or that has trailing bytes, a host-side encoding failure, input past `u32::MAX`) or `CallError::Fault`
  (guest code cut off: a trap, the guest's panic, an import error, the epoch deadline). A refusal answers
  unavailable and logs once per member and hook through the store's `refusals` set (`runtime.rs:776-781`); the
  depth limit is a refusal that logs once per call path (`:716-722`); a busy group keeps today's `warn` (`:687`).
  A fault still stops the whole group at `:774-794`, and nothing clears the flag. A refusal during a reply's
  writeback decodes the whole reply before any field is assigned, so the record keeps its old value.
- **The deadline counts wall clock.** `engine()` (`:74-90`) runs a thread that adds one epoch every millisecond. An
  outermost call sets a deadline of 100 epochs, or 30,000 for `Load` (`:587-593`). Nested calls keep the outer
  deadline. Epochs keep advancing while native code runs inside an import and while the guest thread is
  descheduled. The test `nested_load_calls_do_not_extend_the_outer_deadline` advances 1,000 epochs inside an import
  and shows that a finite nested call afterwards stops the group.
- **A stopped Part goes quiet.** The generated `WasmHandler` falls back to `NoAnswer` (`wire/host.rs:244`). The
  block-entity tick (`block_entity_ticks.rs:19-94` → `tick_mod_block_entity`, `native_hooks/shared.rs:3371-3420`)
  keeps iterating and saving the entity, so its mod logic silently never runs again. That silence is arismoko/horizon#2766's bug.
- **What a trap skips in the guest.** `wire_guest::run` pushes a `Keep` and takes a per-depth reply buffer, then
  pops and returns them when the call ends. A trap skips the pop. It also skips every `Drop` and every `RefCell`
  borrow release in the frames it unwinds, and an allocation that was mid-update in the std member's allocator
  stays mid-update. The group state (`horizon_group_state`: the handler table, the cache budget, the eviction
  callbacks) belongs to the std member, so every member shares it.
- **What a fault can't leave half-done on the host.** Under the arismoko/horizon#1909 boundary, the host decodes a reply completely
  before writing any field back, and point records do the same. So a call that faults applies none of its reply.
  What it already did through imports before the fault stays done. The native resource scope and `FrameScope`
  unwind correctly, and the group is marked stopped before held native objects drop (`PoisonOnPanic`).
- **Native differs.** A native listener's panic is fatal to the game, and the crash report names the mod through
  `content/crash.rs`. Wasm stopping only the faulting group is already more lenient. This note keeps that difference,
  because it comes from the sandbox, not from the API.

## 1. Interruption by guest work (arismoko/horizon#2766)

The runaway check should fire when the guest itself does too much work. Waiting on the host or on the OS scheduler
must not count, and a busy server must give the same outcome as an idle one.

**Mechanism: fuel.** `Config::consume_fuel(true)` makes every guest instruction consume fuel from the store's
tank, and a guest that empties it traps with `Trap::OutOfFuel`.
- **Setting the tank.** An outermost call fills it with `Store::set_fuel(budget)`. A nested call (one that resumes
  the store lent to native code, `Resumed::take`) does not refill it. So a reentered callback spends from the same
  tank as the call that reentered it.
- **No accounting on the host side.** Fuel stops being consumed the moment the guest calls an import and resumes
  when it returns. Time spent in native dispatch, in nested native work, waiting for a lock or descheduled is never
  charged. Nested guest work is always charged, once, to the shared tank.
- **What it covers.** Both cases fork-review raised hold by construction:
  - a finite call that waits a long time inside an import, then makes a nested call, survives, because the wait
    used no fuel;
  - a runaway loop inside a nested callback empties the shared tank and is interrupted, however the work is split
    between the outer and nested calls.
- **Budgets.** Fuel counts instructions, not milliseconds, so today's 100 and 30,000 epochs don't carry over. Each
  budget is set from measurement: the most fuel any ordinary call and any `Load` call uses in the in-tree samples and
  fixtures (horizon-machine-sample, the wasm fixture, the group fixtures), with a margin stated next to the
  constant. The 100-epoch value is not evidence for any particular fuel number.
- **Cost.** Fuel instruments guest code. arismoko/horizon#1780's profile shows guest dispatch glue, codecs and allocation are a large
  share of the remaining wasm tick, so the overhead is measured on arismoko/horizon#1780's scene before fuel replaces epochs (see
  "What to measure"). Wasmtime's `fuel_async_yield_interval` and async features are not used.

**Fallback, only if fuel's cost is rejected: a best-effort wall-clock heuristic.** It keeps epochs and a deadline
callback, and counts host time with exclusive transitions so nothing is charged twice:
- the store keeps `host_epochs` and `mark`, the epoch at the last transition;
- **guest → host** (import entry, and a nested call returning into an import) sets `mark = now`;
- **host → guest** (import exit, and a nested call starting inside an import) adds `now - mark` to `host_epochs`,
  so an outer import's time is credited before the reentered callback runs;
- the deadline callback continues by the credited epochs while `elapsed - host_epochs` is under the budget, and
  interrupts once it isn't.

That heuristic still charges descheduled guest time, so a starved finite call can be interrupted. The note says so,
and the acceptance rows for the fallback say so too. It is not offered as a guest-work budget, and no CPU-time check
is added on top of it.

## 2. After an interruption or a trap (arismoko/horizon#2766, arismoko/horizon#2761)

| Failure | Guest code | Group | Call answers | Diagnostic |
|---|---|---|---|---|
| Guest error reply (`u32::MAX`) | returned normally | keeps running | unavailable | once per listener: the mod, hook and message |
| Host can't decode the reply, or it has trailing bytes | returned normally | keeps running | unavailable | once per listener: mod, hook, decode error |
| Depth over 64 | not entered at that depth | keeps running | unavailable | once per call path: the path, the limit |
| Busy | not entered | keeps running | unavailable | as today (`warn`) |
| Trap, guest panic, import error | cut off | **stops** | unavailable | fault report (decision 6) |
| Out of fuel (after §1) | cut off | **stops** | unavailable | fault report, marked runaway, with the budget |

**Why a refusal is safe:** for every refusal, guest code either never ran at that depth or returned through
`wire_guest::run`, which pops its own state. The host decoded nothing, so it wrote nothing back. Rate-limiting the
log keeps a listener that refuses every tick from flooding it. A listener that refuses N times in a row could be
stopped by policy, but that is a choice to make, not a requirement for safety (open question 3).

**Why a fault stops the group:** see §4. A stopped group is skipped before any work, which costs one relaxed load,
as today.

## 3. Attributing a fault to a member (arismoko/horizon#2761)

Today the log names only `self.id`, the member whose hook was entered. That is wrong whenever the fault happens in a
library the addon called, or in a nested callback into another member.

- **The faulting member.** Read the innermost frame of the trap's `WasmBacktrace` (`FrameInfo::module`) and map its
  module back to its member with `Module::same` against the modules the linker compiled (`link.rs:509`). Members are
  linked with `--strip-all`, so function names are missing, but which module a frame belongs to is not. When fuel
  runs out, this is the member that was running as the tank emptied. Stated honestly: it is not necessarily the
  member that used most of the budget.
- **The entered member and the call path.** The host already pushes a frame for each nested call (`cx.frames`,
  `cx.module`). The report lists every member on that stack, outermost first, with the hook or point at each level.
- **What stays unattributable.** Direct calls from one member into a library don't cross the host, so the time
  inside a library function can't be charged to its caller without fuel or a profiler. The report names members at
  the granularity of host crossings plus the innermost frame, and says that this is the limit.
- **A guest panic's message.** The panic hook already sends the text through `HOST_LOG` before the trap. The fault
  report attaches that message to the innermost member, not to `cx.module`.

## 4. What is safe to keep running (arismoko/horizon#2761)

**The constraint:** all members of a group share one linear memory, one std member (with its allocator and handler
table) and one copy of each library. A trap can stop:

- inside the allocator, leaving free lists inconsistent for every member;
- with a `RefCell` borrowed, so a later borrow of it panics in another member;
- partway through a library's static update, which every dependent then reads;
- with `wire_guest`'s per-depth stacks not popped, so the next call at that depth starts out of step.

The trap gives no proof that it avoided all of these. So, after a fault:

- **Resuming any member of the faulted instance is unsafe.** That includes members that "had nothing to do with
  it": the allocator and the handler table are theirs too.
- **Splitting the group to contain faults is rejected.** Giving each addon its own copy of the library would break
  the one-copy model (arismoko/horizon#1909) and make wasm's dependency model differ from native's.
- **Rolling back to a snapshot before each call is rejected for hot paths.** It means copying or tracking writes to
  the group's memory on every outermost call. Wasmtime offers no copy-on-write checkpoint of a live memory, so the
  cost would be per call. It stays an open question for cold calls (`Load`).
- **Rebuilding is safe.** A new store, a new memory and a fresh `Load` for every member except the faulting one and
  its dependents (decision 4). The rebuild runs between ticks, never inside a call. It needs the following, before
  it can be built:
  - **Content registered at load must replay identically.** Registries are frozen once the world loads. The rebuilt
    members' `Load` must make the same registrations, which the host checks. On any difference the group stays
    stopped, with that reason.
  - **Host-held guest handler ids become stale.** Every Part, provider and handler id the host holds for that group
    is released and resolved again from the new instance, by the same ordering of registrations.
  - **State held only in guest memory is lost.** It is lost the same way as on a restart. Data kept in block entity
    NBT or attachments survives.
  - **One rebuild per fault, not a loop.** A member that faults again after its group is rebuilt stays refused until
    the next start.

## 5. Constraints kept

- **One copy of each library per group, in one memory.** A rebuild replaces the instance; it never duplicates a
  library.
- **The same dependency model for native and wasm.** Dropping a faulting member's dependents follows the load-time
  rule for a missing required dependency. No wasm-only packaging unit or grouping is added.
- **The arismoko/horizon#1909 boundary is unchanged.** Wire bytes in, a reply decoded whole before write-back, and refusals at load
  for what can't cross.
- **No silent skip.** Every stop, refusal and rebuild is reported, and the Mods page shows the group's state.

## What to measure and test

All measurements run on the same frozen main, with the same scene as arismoko/horizon#1780's runs: a local bench scene, 1,000
`machine_sample:machine`, a flat dedicated world, 15 s warmup and 20 s report, three repetitions, with the CPU named.

| Measurement | Compared | Acceptance |
|---|---|---|
| Fuel cost | fuel against today's epochs: wasm mspt mean and p99 on the scene, the `block_entity_ticks` bench, and a 1M-operation guest loop | adopted if within the run-to-run spread; otherwise the numbers go to the owner with the fallback |
| Budgets | the most fuel per ordinary and per `Load` call across the in-tree samples and fixtures | the budget constants and their margins come from this |
| Determinism | the same call's fuel use on an idle machine and under a busy sibling pinned to the same core | identical |
| Long import, then nested call | a finite call that blocks 1 s inside an import, then makes a nested finite call | survives, and the group keeps running |
| Starved thread | a finite call whose thread is run under `SCHED_IDLE` beside a busy sibling | survives (fuel); for the fallback, may be interrupted, reported as such |
| Runaway | an infinite loop; an infinite loop in a nested callback; a loop inside a library; a loop split across outer and nested calls | interrupted every time; the report names the innermost member |
| Refusal survival | one case per refusal kind: guest error reply, undecodable reply, trailing bytes, depth 65, busy | each answers unavailable, the group keeps running, and the next call to the same listener succeeds |
| Independent groups | a trap in group A, which has two members; group B is separate | both members of A stop; B keeps answering |
| Transaction abort | a fault after a write through an import inside a transaction, and one outside any transaction | the transaction's write is rolled back; the write outside stays, as decided (8) |
| Fault isolation (after the rebuild lands) | addon trap in a group with a library and a second addon | the second addon and the library answer after the rebuild; the faulting addon stays refused |
| Rebuild cost | time to rebuild a group of 3 and of 10 members between ticks | reported, not gated |

## Open questions

1. **If fuel costs too much, which way?** Accept the cost, take the best-effort fallback and live with load-dependent
   interruptions, or cut guest work first (arismoko/horizon#1780) and measure again. That choice goes to the owner with the numbers.
2. **When is the rebuild allowed?** Between any two ticks, or only at a save point, or on an operator command? A
   rebuild mid-session drops guest-memory state, which some mods may not expect.
3. **Should repeated refusals stop a listener?** For example, after 100 consecutive refusals of the same listener,
   stop that listener only (never the group). It is safe either way; the question is noise against hiding a broken
   mod.
4. **Snapshots for `Load`.** A checkpoint before each `Load` call could let a fault there drop only the faulting
   member without a full rebuild. Is that worth the extra path, or is the rebuild enough?
5. **Writes outside any transaction.** Decision 8 leaves them applied. Should more hooks run their imports inside a
   transaction so a fault can undo them? That belongs to arismoko/horizon#796 and arismoko/horizon#781's transaction contract.
6. **Who sees the report in multiplayer?** Server log and operators, certainly. Should players get a chat line when
   a group whose content they're interacting with stops?
