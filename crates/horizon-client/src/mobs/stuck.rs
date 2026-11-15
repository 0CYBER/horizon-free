//! What is stuck in a living entity's body: `LivingEntity.DATA_ARROW_COUNT_ID`
//! and `DATA_STINGER_COUNT_ID` with the timers that pull them out
//! (`removeArrowTime`, `removeStingerTime`). A mob keeps it in its `Extra`; a
//! player's lives with the combat record, which is swapped with the body it
//! belongs to.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stuck {
    pub arrows: i32,
    pub stingers: i32,
    pub remove_arrow_time: i32,
    pub remove_stinger_time: i32,
}

impl Stuck {
    /// `AbstractArrow.onHitEntity`: a hit that hurt, from an arrow that
    /// does not pierce, leaves it stuck.
    pub fn add_arrow(&mut self) {
        self.arrows = self.arrows.wrapping_add(1);
    }
    /// `Bee.doHurtTarget`: a sting that hurt.
    pub fn add_stinger(&mut self) {
        self.stingers = self.stingers.wrapping_add(1);
    }
    /// The server's part of `LivingEntity.tick`: one in a count's worth of
    /// `20 * (30 - count)` ticks falls out. Whether the counts changed.
    pub fn tick(&mut self) -> bool {
        let (arrows, stingers) = (self.arrows, self.stingers);
        Self::decay(&mut self.arrows, &mut self.remove_arrow_time);
        Self::decay(&mut self.stingers, &mut self.remove_stinger_time);
        (arrows, stingers) != (self.arrows, self.stingers)
    }
    fn decay(count: &mut i32, remove_time: &mut i32) {
        if *count > 0 {
            if *remove_time <= 0 {
                *remove_time = 20i32.wrapping_mul(30i32.wrapping_sub(*count));
            }
            *remove_time = remove_time.wrapping_sub(1);
            if *remove_time <= 0 {
                *count -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Stuck;

    #[test]
    fn arrow_hit_wraps_max_mod_count() {
        let mut s = Stuck {
            arrows: i32::MAX,
            ..Stuck::default()
        };
        s.add_arrow();
        assert_eq!(s.arrows, i32::MIN);
        assert!(!s.tick());
        assert_eq!(s.arrows, i32::MIN);
    }

    #[test]
    fn sting_wraps_max_mod_count() {
        let mut s = Stuck {
            stingers: i32::MAX,
            ..Stuck::default()
        };
        s.add_stinger();
        assert_eq!(s.stingers, i32::MIN);
        assert!(!s.tick());
        assert_eq!(s.stingers, i32::MIN);
    }

    /// `LivingEntity.tick`: with `n` stuck the timer starts at
    /// `20 * (30 - n)` and one falls out when it runs down.
    #[test]
    fn arrows_fall_out_after_twenty_ticks_times_the_gap_to_thirty() {
        let mut s = Stuck::default();
        s.add_arrow();
        s.add_arrow();
        let wait = 20 * (30 - 2);
        for _ in 0..wait - 1 {
            assert!(!s.tick());
        }
        assert!(s.tick());
        assert_eq!(s.arrows, 1);
        // The timer starts again from the new count.
        for _ in 0..20 * (30 - 1) - 1 {
            assert!(!s.tick());
        }
        assert!(s.tick());
        assert_eq!(s.arrows, 0);
        assert!(!s.tick());
    }

    #[test]
    fn stingers_have_their_own_timer() {
        let mut s = Stuck::default();
        s.add_stinger();
        s.add_arrow();
        for _ in 0..20 * 29 - 1 {
            assert!(!s.tick());
        }
        // Both timers run 580 ticks.
        assert!(s.tick());
        assert_eq!((s.arrows, s.stingers), (0, 0));
    }
}
