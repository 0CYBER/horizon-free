//! The adaptive triggers' effects in the pad's own encoding. A trigger's travel is cut into ten
//! zones (0 at rest, 9 fully pulled); an effect names the zones it acts in and a strength, 1..8,
//! for each, three bits per zone. Modes as documented by Nielk1's trigger effect notes and
//! DualSenseX; [`crate::TRIGGER_OFF`] frees the trigger.
use crate::TriggerEffect;

const FEEDBACK: u8 = 0x21;
const WEAPON: u8 = 0x25;
const VIBRATION: u8 = 0x26;

/// Zone bits and three-bit levels (`level - 1`) for the zones whose level isn't 0.
fn zones(levels: [u8; 10]) -> (u16, u32) {
    let mut active = 0;
    let mut values = 0;
    for (i, level) in levels.into_iter().enumerate() {
        if level > 0 {
            active |= 1 << i;
            values |= u32::from(level.min(8) - 1) << (3 * i);
        }
    }
    (active, values)
}

fn zoned(mode: u8, levels: [u8; 10]) -> TriggerEffect {
    let (active, values) = zones(levels);
    let mut e = [0; 11];
    e[0] = mode;
    e[1..3].copy_from_slice(&active.to_le_bytes());
    e[3..7].copy_from_slice(&values.to_le_bytes());
    e
}

/// Resistance in each zone, 0 (none) to 8; it holds wherever the trigger is.
pub fn resistance(levels: [u8; 10]) -> TriggerEffect {
    zoned(FEEDBACK, levels)
}

/// Even resistance from zone `start` (0..9) to the end of the travel, at `strength` (1..8).
pub fn feedback(start: u8, strength: u8) -> TriggerEffect {
    resistance(std::array::from_fn(|i| {
        if i >= usize::from(start) {
            strength.clamp(1, 8)
        } else {
            0
        }
    }))
}

/// A gun trigger: resistance from zone `start` that gives way with a click at zone `end`
/// (`2 <= start < end <= 8`), at `strength` (1..8).
pub fn weapon(start: u8, end: u8, strength: u8) -> TriggerEffect {
    let start = start.clamp(2, 7);
    let end = end.clamp(start + 1, 8);
    let mut e = [0; 11];
    e[0] = WEAPON;
    e[1..3].copy_from_slice(&((1u16 << start) | (1u16 << end)).to_le_bytes());
    e[3] = strength.clamp(1, 8) - 1;
    e
}

/// A buzz from zone `start` (0..9) on, at `amplitude` (1..8) and `frequency` Hz.
pub fn vibration(start: u8, amplitude: u8, frequency: u8) -> TriggerEffect {
    let mut e = zoned(
        VIBRATION,
        std::array::from_fn(|i| {
            if i >= usize::from(start) {
                amplitude.clamp(1, 8)
            } else {
                0
            }
        }),
    );
    e[9] = frequency;
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_bytes() {
        // Zones 3..9 at strength 6 (stored as 5, three bits each).
        assert_eq!(
            feedback(3, 6),
            [0x21, 0xf8, 0x03, 0x00, 0xda, 0xb6, 0x2d, 0, 0, 0, 0]
        );
        assert_eq!(feedback(0, 1), [0x21, 0xff, 0x03, 0, 0, 0, 0, 0, 0, 0, 0]);
        // Out-of-range strengths are clamped.
        assert_eq!(feedback(0, 0), feedback(0, 1));
        assert_eq!(feedback(0, 9), feedback(0, 8));
    }

    #[test]
    fn resistance_per_zone() {
        let e = resistance([0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(u16::from_le_bytes([e[1], e[2]]), 0b11_1111_1100);
        let values = u32::from_le_bytes([e[3], e[4], e[5], e[6]]);
        for zone in 2..10 {
            assert_eq!((values >> (3 * zone)) & 7, zone - 2);
        }
    }

    #[test]
    fn weapon_bytes() {
        assert_eq!(
            weapon(2, 6, 6),
            [0x25, 0x44, 0x00, 0x05, 0, 0, 0, 0, 0, 0, 0]
        );
        // The click is always past the start.
        assert_eq!(weapon(7, 3, 8)[1..3], [0x80, 0x01]);
    }

    #[test]
    fn vibration_bytes() {
        let e = vibration(8, 2, 30);
        assert_eq!(e[..3], [0x26, 0x00, 0x03]);
        assert_eq!(
            u32::from_le_bytes([e[3], e[4], e[5], e[6]]),
            1 << 24 | 1 << 27
        );
        assert_eq!(e[9], 30);
    }
}
