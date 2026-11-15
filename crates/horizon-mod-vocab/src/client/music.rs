//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `Music`: a sound event and the delay range (ticks) before it starts.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct Music {
    pub sound: String,
    pub min_delay: i32,
    pub max_delay: i32,
    pub replace_current_music: bool,
}
impl Music {
    pub fn new(sound: &str, min_delay: i32, max_delay: i32, replace_current_music: bool) -> Self {
        Self {
            sound: sound.to_owned(),
            min_delay,
            max_delay,
            replace_current_music,
        }
    }
    /// `Musics.MENU`.
    pub fn menu() -> Self {
        Self::new("minecraft:music.menu", 20, 600, true)
    }
    /// `Musics.CREDITS`.
    pub fn credits() -> Self {
        Self::new("minecraft:music.credits", 0, 0, true)
    }
    /// `Musics.END_BOSS`.
    pub fn end_boss() -> Self {
        Self::new("minecraft:music.dragon", 0, 0, true)
    }
    /// `Musics.CREATIVE`.
    pub fn creative() -> Self {
        Self::new("minecraft:music.creative", 12000, 24000, false)
    }
    /// `Musics.END`.
    pub fn end() -> Self {
        Self::new("minecraft:music.end", 6000, 24000, true)
    }
    /// `Musics.UNDER_WATER`.
    pub fn under_water() -> Self {
        Self::new("minecraft:music.under_water", 12000, 24000, false)
    }
    /// `Musics.GAME`.
    pub fn game() -> Self {
        Self::new("minecraft:music.game", 12000, 24000, false)
    }
    /// The built-in situations the Music page lists, in its order: the lang key
    /// of the heading the page's group shows and the music that situation plays.
    pub fn builtin() -> [(&'static str, Music); 7] {
        [
            ("horizon.pages.music.menu", Self::menu()),
            ("horizon.pages.music.overworld", Self::game()),
            ("horizon.pages.music.creative", Self::creative()),
            ("horizon.pages.music.underwater", Self::under_water()),
            ("horizon.pages.music.end", Self::end()),
            ("horizon.pages.music.boss", Self::end_boss()),
            ("horizon.pages.music.credits", Self::credits()),
        ]
    }
    /// `Music.CODEC`.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn parse(v: &Value) -> Option<Self> {
        let sound = v["sound"].as_str()?;
        let int = |k: &str| v[k].as_i64().unwrap_or(0) as i32;
        Some(Self::new(
            sound,
            int("min_delay"),
            int("max_delay"),
            v["replace_current_music"].as_bool().unwrap_or(false),
        ))
    }
}
/// A track a native mod plays over or instead of the music
/// (`music_layers`): a sound event and its volume, 0 to 1.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct MusicLayer {
    pub sound: String,
    pub volume: f32,
}
