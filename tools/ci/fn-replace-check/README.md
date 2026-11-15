Two native mods whose `[[native.mixin]]` rows the launcher builds against the prebuilt game by function
replacement (no game rebuild): `tools/ci/fn_replace_check.sh` builds them and runs a dedicated server.
Every helper prints a line starting `fnr:` so the run shows what ran, in what order.

`loop_generic_head` is built on its own, never beside `loop`: a HEAD mixin on the published loop's generic
function with no `[[native.loop]]` row, so the copy is the game's own loop with the empty slot.

`loop_second` is built beside `loop`, in the workflow's main set: a second mod's ticking block entities in the same world, and a slot of
its own that counts only those.

`api_only` has hooks and an `[[inject]]` listener and no `[native]` table row. Alone (`FNR_ROUTE=prebuilt-api`) it
takes the API route: the package's prebuilt API executable with the launcher's `horizon_native_mods` library. In the
main set, beside mods with `[[native.mixin]]` rows (`FNR_ROUTE=prebuilt`), the whole set takes the engine route.
