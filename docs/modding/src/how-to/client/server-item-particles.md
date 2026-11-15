# Server item particles

Construct `ParticleOptions::item_stack(item_particle_key, stack)` and call
`level.send_particles(host, &options, state, at, spread, speed, count)` with a
`LevelRef` and a `ServerHost`. The key is the
load-resolved `minecraft:item` particle key; the stack is an API `ItemStack`.
The same request crosses the native and wasm boundaries through `Wire`.

The server validates the stack against the registered items and component and
count limits. Invalid stacks return `false` without queueing a particle. A `true`
result means queued, not necessarily drawn. The ordinary particle packet carries
the item stack, not server-selected sprite names. Each client resolves its ground
item model with seed 0, through the same path as
`ClientHost::item_particle_options`, so its resource pack and stack model
components determine the layer particle sprites.

The resolved `ParticleOptions::item` sprite list is for local rendering; it is
not a substitute for the stack on server sends. Block, block-marker, falling-dust,
dust-pillar and block-crumble requests require a valid block state argument.
Trail and vibration requests are refused by the server send.
