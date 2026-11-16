//! Which level-control hooks this thread is hearing, so a mod's setter called from inside its
//! own hook is refused instead of firing that hook again without end: `weather_change` and
//! `thunder_change` for `set_weather`, `world_border_center_changed` and
//! `world_border_bounds_changed` for the border's centre and size, `server_tick_rate_changed`
//! for `set_tick_rate`. Each setter firing a hook marks its own control, and a nested setter
//! of another control can't come back to it, so a chain of listeners ends after at most one
//! write per control. Commands keep vanilla's behaviour: `/tick rate` from a listener still
//! runs and is heard again.
use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum LevelControl {
    Weather,
    BorderCenter,
    BorderBounds,
    TickRate,
}

thread_local! {
    static HEARING: Cell<u8> = const { Cell::new(0) };
}

impl LevelControl {
    const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Whether this control's hook is being heard on this thread.
    pub(in crate::game) fn is_heard(self) -> bool {
        HEARING.with(|h| h.get() & self.bit() != 0)
    }

    /// Runs `f`, a dispatch of this control's hook, with the control marked as heard until it
    /// returns or unwinds.
    pub(in crate::game) fn hear<R>(self, f: impl FnOnce() -> R) -> R {
        struct Heard(u8);
        impl Drop for Heard {
            fn drop(&mut self) {
                HEARING.with(|h| h.set(self.0));
            }
        }
        let _restore = Heard(HEARING.with(|h| h.replace(h.get() | self.bit())));
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::LevelControl;

    #[test]
    fn a_control_is_heard_only_inside_its_own_dispatch_and_after_an_unwind_is_not() {
        assert!(!LevelControl::Weather.is_heard());
        LevelControl::Weather.hear(|| {
            assert!(LevelControl::Weather.is_heard());
            assert!(!LevelControl::TickRate.is_heard());
            LevelControl::TickRate.hear(|| assert!(LevelControl::Weather.is_heard()));
            assert!(!LevelControl::TickRate.is_heard());
        });
        assert!(!LevelControl::Weather.is_heard());
        let unwound = std::panic::catch_unwind(|| {
            LevelControl::BorderCenter.hear(|| panic!("listener"));
        });
        assert!(unwound.is_err());
        assert!(!LevelControl::BorderCenter.is_heard());
    }
}
