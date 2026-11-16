# Turning hanging entities

You can turn a painting, item frame or glow item frame where it hangs, instead of breaking it and placing a
new one. Both writes are server-only handle methods on `EntityRef`, taking `&mut dyn ServerHost`:

- `set_hanging_direction(host, direction)` is `HangingEntity.setDirection`: only the facing changes, and the yaw,
  pitch and bounds follow from it. Nothing checks that the entity can hang there; the next survival check does,
  as in vanilla. An item frame takes all six directions, a painting only the four horizontal ones. Vanilla's
  `Painting.setDirection` throws on up or down; this API refuses them instead: it returns `false` and changes nothing.
- `set_item_frame_rotation(host, rotation)` is `ItemFrame.setRotation`: the item turns in eighths of a turn, and the
  value wraps modulo 8 as vanilla's does (8 is 0, 13 is 5). Comparators reading the frame update afterwards.

Both return `false` for a stale handle or an entity of another kind. Players see the change through entity synchronization, including a painting or frame turned to south.
