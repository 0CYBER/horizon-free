# Villager profession resources and clients

Profession layers use the registered profession's qualified identity. For
`test:engineer`, provide
`assets/test/textures/entity/villager/profession/engineer.png` and
`assets/test/textures/entity/zombie_villager/profession/engineer.png`.
Nested identity paths remain nested texture paths. Vanilla professions use the
same resource lookup.

The texture's `.png.mcmeta` supplies its hat policy:

```json
{"villager":{"hat":"full"}}
```

`none` keeps the type layer's head visible, `full` hides it, and `partial`
hides it only when the type texture has a full hat. Missing metadata means
`none`. Pack reload refreshes the cached metadata. Unresolved saved professions
retain their names and render the neutral base without a profession layer.

Require the mods that declare professions on both sides, with matching versions.
The configuration mod-list handshake refuses a missing mod by name before play
and entity pairing. Both sides resolve vanilla's `VillagerData` integer profession
index against their local deterministic table. An unresolved server identity
projects to `none` on the network without changing the saved identity.
