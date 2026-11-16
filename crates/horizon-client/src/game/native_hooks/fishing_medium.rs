/// Where a fishing bobber outside water floats and fishes, as a mod's
/// `native_fishing_medium` answers it: lava, the void under an island, a
/// cloud. Below `surface` the bobber floats and fishes as in still water,
/// above it it falls as anywhere else. API @API:fishing_medium@.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::game) struct NativeFishingMedium {
    /// The height the bobber floats at (world Y), as water's surface is.
    pub surface: f64,
    /// The loot table a catch from here rolls (`mymod:gameplay/lava_fishing`);
    /// None rolls vanilla's `minecraft:gameplay/fishing`.
    pub loot_table: Option<String>,
    /// The ticks a fish takes to come, drawn from `min..=max` when the bobber
    /// starts waiting, less the rod's Lure as in water (vanilla draws 100 to
    /// 600). None keeps vanilla's draw.
    pub lure_ticks: Option<(i32, i32)>,
}

impl super::Game<'_> {
    // API exposure is held until a shared native/wasm representation is approved.
    pub(in crate::game) fn native_fishing_medium(
        &mut self,
        _dimension: &str,
        _hook: u32,
        _position: [f64; 3],
        _owner: Option<[f64; 3]>,
        _medium: &mut Option<NativeFishingMedium>,
    ) {
    }
}
