# Container loot

`container_loot_filled` is a server-side Horizon addition. It exposes the rolled list in roll order, after table-load changes and before shuffle, splitting and placement. Listeners run in dispatch order and may add, remove or replace stacks. Empty stacks disappear; oversized stacks are trimmed to the effective item maximum before placement, and excess stacks that cannot fit are discarded.

The context names the table, dimension, block position or entity id, and the block centre or vehicle position. `opener` is present only when a player's open starts the fill. Hopper, comparator, break, explosion and command accesses have no opener. Each half of a double chest fills separately. Already-filled containers and block or mob drops do not fire this hook. With no listener, no API values are built and random draws are unchanged.

Listeners use their own randomness. Invalid component writes retain the prior list and produce the standard hook refusal warning. A panicking listener follows the mod runtime's normal panic handling.
