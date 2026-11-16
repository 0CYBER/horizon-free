<!-- chapter: Collision exclusions -->
<!-- separator: 1 -->

### Collision exclusions

A portal in a wall is a hole only in the picture: the wall behind it still stops the player who walks in. A mod opens it
with collision exclusions, boxes of the player's own level (`[min_x, min_y, min_z, max_x, max_y, max_z]`) in which that
level's blocks don't stop the player, as Immersive Portals' `CollisionHelper` does:

- the part of a block's collision shape inside a box doesn't collide (`Shapes.join(shape, box, ONLY_FIRST)`), so a box
  over half a block leaves the other half solid;
- a block sharing volume with a box is no wall to suffocate in (`isInWall`), the client doesn't push its player
  out of it (`moveTowardsClosestSpace`), and it doesn't act on a player inside it (`checkInsideBlocks`: no cobweb,
  powder snow or damaging block there);
- entity colliders (boats, shulkers) and the world border still stop the player.

There are two halves, and **the mod sets both**: the engine sends neither set to the other side.

- On the server, `player.entity().set_collision_exclusions(host, &boxes)` (`EntityRef`, `&mut dyn ServerHost`). The
  server checks the player's moves through these boxes, so a move through an excluded wall isn't sent back.
- On the client, `host.set_collision_exclusions(&boxes)` (`ClientHost`), for the player the client moves itself.

A client half with boxes the server half doesn't have walks through a wall the server sends it back from; the other
way round, the client stops at a wall the server would have let it through.

Each call replaces the last set, and an empty slice clears it. A change of level, a respawn and leaving clear it too,
on each side; a mod that still wants boxes after a respawn sets them again (the client's `client_player_clone` hook is
the moment). Nothing is saved. Only players keep a set.

A call is refused with a `CollisionExclusionsError`, and the set there was stays: `TooMany` for more than 64 boxes,
`NotFinite` for a value that is NaN or infinite, `Inverted` for a box whose minimum is above its maximum on an axis,
`NotAPlayer` for another entity, and `Stale` for a stale handle or a client with no player of its own.

A player with no boxes pays one empty-slice check per move. With boxes, each block shape a box reaches is joined
with it, which allocates, and the server copies the guest's set into its body every turn.
