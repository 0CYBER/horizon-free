//! The `map_decorations` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0146

pub trait Hooks {
    /// Icons a mod puts on map `map` each time it is drawn: held, in an
    /// item frame or on the cartography table's screen (a map's pixels are
    /// its own; nothing is saved in the map). Push `MapDecoration`s
    /// (an atlas sprite, a position in half map pixels, a rotation, and
    /// whether it shows in a frame and on the screen like vanilla's markers
    /// do); they are drawn after the vanilla ones with the same pose, and a
    /// sprite the atlas lacks is left out. `map.center`, `map.scale` and
    /// `map.dimension` say which blocks the map shows, and
    /// `MapDecoration::x` has the formula. Vanilla's decorations are
    /// not passed in. Asked every frame a map is drawn, so keep it cheap.
    /// The client half of `MapDecorationType` and `addDecoration`: a type
    /// registered on the server and saved in the map's data is not here.
    #[hook(
        side = "client",
        contract = "collect",
        out = "decorations",
        neoforge = "MapDecorationType",
        cancellable = false
    )]
    fn map_decorations();
}
