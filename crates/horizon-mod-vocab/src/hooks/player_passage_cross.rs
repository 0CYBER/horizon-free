//! The `player_passage_cross` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player`'s client crossed `passage` (#3775), one of the player's level's, from `from`
    /// (its feet before), and the server is about to follow: the move to `from` passed the
    /// move checks and both ends are tracked for the player, and the server has not yet
    /// worked out where the player comes out. Set `cancel` and the server refuses the
    /// crossing (`PassageCrossRefusal::Vetoed`) and puts the player back at `from`, as a
    /// portal Immersive Portals' `canTeleportEntity` turns down does. A listener that
    /// removes the passage or teleports the player refuses it too. Server-side. Diverges
    /// from upstream: new hook.
    #[notes = r###"### Passages within a level

`crates/horizon-client/src/game/modding/native_passages.rs` and `native_passage_cross.rs`, with the hub's half in
`crates/horizon-client/src/lan/hub/passage_cross.rs` (#3775, docs/modding/src/how-to/passages.md).

- `cross_passage(passage)` (client) moves the window's player through a passage its server told it of (feet, last
  tick's place, velocity, look, turned) and sends `horizon:passage_cross` (the number, the feet before). Refused with
  nothing moved or sent as a `PassageCrossError`; a payload a mod's `client_packet_send` cancels is `Cancelled`. `last_passage_refusal()` (client) is the server's
  `PassageCrossRefusal` of the last crossing, until the next.
- On the server the move to where it crossed from is checked as any move; the crossing then holds the guest's
  other packets until the game has decided it from its own `from`. The game checks eligibility, the passage, the
  20-block reach and the opening's and destination's chunks, fires `player_passage_cross`, checks all of it again,
  and only then works out where the player comes out (that chunk must be held too). Accepted with an all-relative,
  zero position packet; refused by name (`horizon:passage_sync`) and put back with an absolute one.


- **`player_passage_cross(player, passage, from, cancel)`**: `player`'s client crossed `passage` from `from` (its feet before), the move there passed and both ends are tracked; the server has not worked out where the player comes out yet. Set `cancel` and the crossing is refused (`PassageCrossRefusal::Vetoed`), the player put back at `from`. Everything checked before is checked again after the listeners, so one that removes the passage or moves the player refuses it too. A passage into another level (`destination_level`) never reaches the hook: the server refuses it `PassageCrossRefusal::FarLevel` first."###]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Immersive Portals' Portal.canTeleportEntity)"
    )]
    fn player_passage_cross();
}
