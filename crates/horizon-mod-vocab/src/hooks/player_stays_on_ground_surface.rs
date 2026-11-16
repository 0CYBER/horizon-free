//! The `player_stays_on_ground_surface` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The player's crouch edge backoff (`Player.maybeBackOffFromEdge`) asks whether the player is staying on the ground surface (`Player.isStayingOnGroundSurface`, vanilla: the shift key is down). `entity` is the player's id; a mod may change `staying`, so a mod can make the player hold to an edge without crouching, or walk off while crouching. Fires on both sides, on the client's replayed ticks too, with the loop `context` of the side that ticks the player (its registries, settings and mod list): register the mod on both, and keep the listener a pure function of its inputs so prediction and the server agree. Beyond NeoForge: there is no counterpart. Diverges from upstream: new hook.
    #[notes = r#"## Player edge backoff

- **`player_stays_on_ground_surface(context, entity, staying)`**: edit whether the player counts as staying on the ground surface (`Player.isStayingOnGroundSurface`, vanilla: the shift key is down) where `maybeBackOffFromEdge` asks, so a mod can make the player hold to an edge without crouching, or walk off one while crouching. It fires on both sides and on the client's replayed ticks, with the loop `context` of the side that ticks the player (see [The loop context](#the-loop-context)), so a mod registers on both and keeps the listener a pure function of its inputs. With no listener the cost is one `listened` branch per movement. Beyond NeoForge (no counterpart); Diverges from upstream: new hook. Mobs never back off an edge, so there is no mob call site."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::motion::native_player_stays_on_ground_surface",
        neoforge = "none (vanilla Player.isStayingOnGroundSurface; NeoForge adds no event or extension for it)"
    )]
    fn player_stays_on_ground_surface();
}
