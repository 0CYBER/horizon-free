#[doc = r#"a muffle of the game's sounds that fades back (`native_muffle_sounds`), as ears ringing after a blast hear them."#]
pub trait Notes {
    #[notes = r#"`native_muffle_sounds(amount, lowpass, ticks, exempt)` (`host.muffle_sounds`) turns every sound down and low-passes it, fading back over `ticks` client ticks, except the sound events in `exempt`. Bundles need "hooks": @API:muffle_sounds@."#]
    fn muffle_sounds_note();
}
