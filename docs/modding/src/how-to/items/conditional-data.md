# Conditional data

Add a `neoforge:conditions` array to control whether a server JSON file loads.
Supported readers are
recipes, loot tables, item modifiers, predicates, enchantments, jukebox songs,
instruments, damage types, advancements, synchronized registry JSON, world-table
tags (including loot, damage, synchronized registry and function tags), and native item/block data maps.
Every condition must hold; a false condition omits the file. Mods can
[register conditions](load-conditions.md) for this same evaluator.
Register one handler per id using the read-only `RegistrationContext`. A handler
reads the condition's JSON from a borrowed raw value and validates its own
arguments. Invalid mod input must return `ConditionError`, never panic.

Ordinary JSON resources select the winning pack file first, then evaluate its
conditions before decoding the data. If the winner is omitted, a lower pack's
version does not return. Tags and data maps evaluate each layer before merging,
`replace`, or `remove`, so an omitted layer leaves lower entries intact. A damage
registry is frozen at world open; reloading reevaluates its tags, not its type
definitions. Generated item default components retain their prototype
lifetime and are not conditional data files.

`neoforge:mod_loaded` asks the owning resource generation's captured enabled
server-mod IDs, including active code-only native and WebAssembly mods, plus
`minecraft` always. Client-only, disabled and refused packages are absent;
resource namespaces are not mod IDs. Other supported forms are `neoforge:true`,
`neoforge:not` (`value`), and `neoforge:and` / `neoforge:or` (`values`). A namespace
is owned only when a loaded mod registered at least one condition in it. Unknown
names there refuse reload, naming the file, condition and mod, at any depth in
these trees.
Foreign unknown names leave the file
absent and produce one ERROR-level log entry per candidate reload naming the
file and its distinct unknown ids (the vanilla view outside a reload logs per
evaluation). Every branch of `not`, `and` and `or` is visited, at every depth:
`or(true, othermod:unknown)` is absent even though its first branch is true.
Foreign unknown ids do not themselves refuse the whole reload.
A file without the conditions array loads normally. Successful reloads publish
changed enabled IDs together with the data generation, including command predicates
and item modifiers; retained readers keep
their old snapshot. Callback errors also refuse publication and keep the previous
usable generation. Native and wasm registrations use the same route.
