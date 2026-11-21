//! A thrown potion's break sound on an older release's server.
//!
//! Before 26.x the splash event (`LevelEvent` 2002, 2007 for a potion with instant effects) carried
//! the glass sound: the client's `LevelEventHandler` played `entity.splash_potion.break` with the
//! particles. 26.3 sends that sound as its own event (1053, 1054) and the splash draws only
//! particles, so a 26.3 client on an older server breaks potions in silence. The sound is played
//! with the splash here, and a separate sound for the same place right after it, from a server
//! that sends both, is the same break's and is skipped.

/// What a client does with an older server's level events, by where its last splash was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SplashSound {
    /// Where the last splash was, whose sound the client played itself.
    at: Option<[i32; 3]>,
}

/// What to do with one level event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplashAnswer {
    /// The sound event to play at the same place first, for a splash.
    pub sound: Option<i32>,
    /// Whether to apply the event itself (false: a sound already played with its splash).
    pub apply: bool,
}

impl SplashSound {
    /// Level event `id` at `pos` from an older release's server.
    pub fn event(&mut self, id: i32, pos: [i32; 3]) -> SplashAnswer {
        let sound = match id {
            2002 => 1053,
            2007 => 1054,
            1053 | 1054 => {
                let apply = self.at.take() != Some(pos);
                return SplashAnswer { sound: None, apply };
            }
            _ => {
                return SplashAnswer {
                    sound: None,
                    apply: true,
                };
            }
        };
        self.at = Some(pos);
        SplashAnswer {
            sound: Some(sound),
            apply: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: [i32; 3] = [4, 64, -2];

    #[test]
    fn a_splash_brings_its_sound_once_whether_the_server_sends_it_or_not() {
        let mut splash = SplashSound::default();
        // A server before the split: the splash alone, then the client plays the sound.
        assert_eq!(
            splash.event(2002, AT),
            SplashAnswer {
                sound: Some(1053),
                apply: true
            }
        );
        assert_eq!(
            splash.event(2007, AT),
            SplashAnswer {
                sound: Some(1054),
                apply: true
            }
        );
        // A server that also sends the sound: that one is skipped, the next is heard.
        assert_eq!(
            splash.event(1054, AT),
            SplashAnswer {
                sound: None,
                apply: false
            }
        );
        assert_eq!(
            splash.event(1054, AT),
            SplashAnswer {
                sound: None,
                apply: true
            }
        );
        // A sound elsewhere is another break's; other events pass untouched.
        splash.event(2002, AT);
        assert!(splash.event(1053, [0, 0, 0]).apply);
        assert_eq!(
            splash.event(1009, AT),
            SplashAnswer {
                sound: None,
                apply: true
            }
        );
    }
}
