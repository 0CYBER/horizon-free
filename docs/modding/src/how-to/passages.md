# Passages within a level

A passage is an opening in a level that leads somewhere else in the same level, as an Immersive Portals portal does. A mod's server side registers it with its level; the engine numbers it and tells every Horizon client that tracks the passage's chunk. A client mod then moves the player through it, and the server follows. A passage may name another open level as its destination, but a player cannot cross into it yet (see [Into another level](#into-another-level)).

```rust,ignore
let passage = mod_api::Passage {
    origin: mod_api::Vec3::new(0.5, 65.5, 8.5), // the opening's centre
    axis_w: mod_api::Vec3::new(1.0, 0.0, 0.0),
    axis_h: mod_api::Vec3::new(0.0, 1.0, 0.0),
    width: 2.0,
    height: 3.0,
    destination: mod_api::Vec3::new(100.5, 65.5, 8.5), // where the centre comes out
    turn: mod_api::PassageTurn::Clockwise90,
    destination_level: None, // the passage's own level; `Some(key)` for another open level
    ..mod_api::Passage::default()
};
match level.register_passage(host, passage) {
    Ok(id) => { /* keep `id` to remove it with `level.remove_passage(host, id)` */ }
    Err(refused) => log::warn!("portal refused: {refused:?}"),
}
```

- **The shape.** `origin` is the centre of a rectangle, `width` blocks along `axis_w` and `height` along `axis_h`. Both axes are unit vectors at a right angle. The far side is the same rectangle centred on `destination`, turned about the vertical axis by `turn`: a clockwise quarter turn takes south (+z) to west (-x), as vanilla's `Rotation` does. A passage leads one way, so a way back is a second passage.
- **Refusals.** Registration returns a `PassageRegistrationError` and registers nothing:
  - `Stale`: the level handle is from an earlier open of the world.
  - `Unavailable`: the call came from a client.
  - `NotFinite`: a coordinate or a size is NaN or infinite.
  - `BadAxes`: an axis is not a unit vector, or the axes are not at a right angle.
  - `BadSize { max: 32 }`: a side is not above 0 and at most 32 blocks.
  - `OutsideWorld`: a corner at either end is past the positions vanilla's server accepts from a client: 30,000,000 blocks across, 20,000,000 up or down.
  - `LevelFull { limit: 256 }` and `ChunkFull { limit: 8 }`.
  - `NoIdsLeft`: every id was given (`u64::MAX` of them), and an id is never given twice.
  - `UnknownLevel`: `destination_level` is a level that is not open, or the passage's own level.
- **Caps.** A level holds 256 passages and a chunk 8. The chunk is the one that holds `origin` (`passage.chunk()`). A client is sent every passage of the chunks it tracks, so the level cap bounds what a client holds, whatever its view distance. The chunk cap bounds what goes out with one chunk, and how many passages a client looks through around a moving player.
- **Who hears of it.** At the end of the server's tick, a registration goes to every Horizon client that tracks the passage's chunk (`ChunkMap.isChunkTracked`). A client that gets the chunk later is sent the passage right after the chunk, at the chunk's `chunk_sent`. A client forgets a passage with its chunk, and a removal reaches the clients that still track the chunk. Vanilla clients are never sent passages.
- **Lifetime.** A passage holds until `remove_passage`, until its level closes, or until the world closes; a failed open drops what the open registered. Passages are not saved, and a datapack reload does not bring them back: a mod registers its own again in `level_load`. Numbers are never given again while the game runs. `remove_passage` returns false for an id from another level, an unknown id, a stale handle and a client.
- **Churn.** A passage registered and removed before the end of the tick is never sent. The changes the server keeps for its clients are bounded by the live passages and the removed ones it already told.
- **On the client.** A client keeps a passage only if it passes the same checks as a registration, its chunk is one the client holds, and it has fewer than 256. It drops the rest and logs why.
- **Cost.** Nothing while no passage was registered. From the first one on, until the world closes, the hub records the chunks it sends, as it does while a mod listens to `chunk_watch`. That is a `Watch` entry and a `Sent` entry for each chunk sent, an `Unwatch` entry for each chunk a player stops watching, and a map lookup for each sent chunk's passages.

## Crossing

A client mod decides when its player goes through, for example when the eye's move of a tick passes the opening, and calls `host.cross_passage(id)` with a passage its server told it of:

```rust,ignore
match host.cross_passage(id) {
    Ok(()) => { /* the player is on the far side now */ }
    Err(refused) => log::debug!("not crossed: {refused:?}"),
}
```

- **On the client.** The player's feet, where they were a tick ago (so the frames between ticks do not jump), its velocity and its look come out at the far side, turned with the passage, as Immersive Portals' client moves its player. Nothing else changes: on ground, fall distance, sprinting, the item in use. The server is sent the passage's number and where the feet were. A `PassageCrossError` refuses with nothing moved or sent: `Unavailable` (not connected to a server), `NoPlayer`, `Pending` (the last crossing still waits for the server's answer), `Riding`, `UnknownPassage` (the window holds no such passage) `TooFar { limit: 20 }` (the feet are more than 20 blocks from the nearest point of the opening, Immersive Portals' limit) and `Cancelled` (a mod's `client_packet_send` cancelled the payload to the server).
- **On the server.** The move to where the client crossed from is checked as any move, with vanilla's numbers. The rest of the player's packets then wait until the server has decided this crossing, from the place it names. The server checks that the player may cross now and is where it crossed from, looks the passage up in the player's level, checks the 20 blocks, and checks that the opening's chunk and the destination's are loaded and tracked for the player. The server-only hook `player_passage_cross` then hears the crossing and may cancel it. After the hook the server checks all of that again, so a listener that removes the passage or moves the player refuses the crossing. Only then does it work out where the player comes out, never from the client's word, and checks that chunk too. It moves the player there, turns its look and motion, and starts its move checks from there.
- **The answer.** The server accepts with a position packet whose position, look and motion are all relative and zero. The client is already there: it stays put, keeps its velocity, and confirms. That confirmation is checked as a move from the far side, off the ground, so a passage that ends in the air passes. Like any position packet, the acceptance stops the player's mining: a passage crossed while digging aborts the dig, and the attack strength starts again. Fall distance carries across, and so does the server's count of ticks spent floating.
- **Refused.** The server tells the client a `PassageCrossRefusal`, which `host.last_passage_refusal()` returns until the next crossing, and puts the player back where the server has them with a position packet. The refusals are `UnknownPassage`, `Pending` (a teleport or an earlier crossing waits for the client's answer), `NotLoaded`, `Dead`, `Sleeping`, `Riding`, `MoveRefused` (the move to where it crossed from failed the move checks), `TooFar { limit: 20 }`, `NearEndNotTracked`, `FarEndNotTracked`, `Vetoed` and `FarLevel` (the passage leads into another level).
- **One at a time.** A player has one crossing pending at most. The client refuses a second one with `Pending` until the server's position packet comes, and the server refuses one that arrives while a teleport waits.

## Into another level

`destination_level: Some(key)` makes `destination` a point in that level. Registration refuses a level that is not open, and the passage's own level, with `UnknownLevel`; leave the field `None` for a passage within its level. Such a passage is registered and can be removed as any other, but until clients can follow a player into another level it is not sent to them, and a crossing of it is refused `PassageCrossRefusal::FarLevel` on the server, with the player put back. A client never holds it, so `host.cross_passage(id)` on the client returns `UnknownPassage`.
