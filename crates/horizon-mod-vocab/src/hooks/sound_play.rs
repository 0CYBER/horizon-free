//! Upstream prose, retained with fork routing described in the modding book.
pub trait Notes {
    #[notes = "`sound_play` intercepts actual client sound starts and live parameter updates. `SoundPlayback` identifies the instance, event, source, position, relative flag and whether it is starting. Edit base volume/pitch transiently or cancel before subtitles/backend writes; invalid edits cancel. No Host/world borrow or reverb/DSP is supplied. Runs on the game thread; use existing client hooks for world/acoustics state. Bundles need \"hooks\": @API:sound_play@."]
    fn sound_play();
}
