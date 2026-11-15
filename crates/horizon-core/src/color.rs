//! Shared vanilla colour calculations.

/// `Mth.hsvToArgb`: packed ARGB, with Java's float arithmetic and truncation.
pub fn hsv_to_argb(hue: f32, saturation: f32, value: f32, alpha: u32) -> u32 {
    let h = (hue * 6.0) as i32 % 6;
    let f = hue * 6.0 - h as f32;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - f * saturation);
    let t = value * (1.0 - (1.0 - f) * saturation);
    let (r, g, b) = match h {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        5 => (value, p, q),
        _ => panic!("invalid HSV hue"),
    };
    let c = |v: f32| ((v * 255.0) as i32).clamp(0, 255) as u32;
    (alpha & 255) << 24 | c(r) << 16 | c(g) << 8 | c(b)
}

/// `ExtraCodecs.RGB_COLOR_CODEC`: boxed int or exactly three float channels.
/// Integers retain all bits; vector channels are floored and masked, not clamped.
pub fn rgb_color_from_tag(value: &crate::nbt::Tag) -> Option<i32> {
    use crate::nbt::Tag;
    let float = |value: &Tag| {
        Some(match value {
            Tag::Byte(n) => f32::from(*n),
            Tag::Short(n) => f32::from(*n),
            Tag::Int(n) => *n as f32,
            Tag::Long(n) => *n as f32,
            Tag::Float(n) => f32::from_bits(*n),
            Tag::Double(n) => f64::from_bits(*n) as f32,
            _ => return None,
        })
    };
    let channels = match value {
        Tag::Byte(n) => return Some(i32::from(*n)),
        Tag::Short(n) => return Some(i32::from(*n)),
        Tag::Int(n) => return Some(*n),
        Tag::Long(n) => return Some(*n as i32),
        Tag::Float(n) => return Some(f32::from_bits(*n) as i32),
        Tag::Double(n) => return Some(f64::from_bits(*n) as i32),
        Tag::List(values) if values.len() == 3 => {
            [float(&values[0])?, float(&values[1])?, float(&values[2])?]
        }
        Tag::ByteArray(values) if values.len() == 3 => {
            [values[0], values[1], values[2]].map(|n| f32::from(n as i8))
        }
        Tag::IntArray(values) if values.len() == 3 => {
            [values[0] as f32, values[1] as f32, values[2] as f32]
        }
        Tag::LongArray(values) if values.len() == 3 => {
            [values[0] as f32, values[1] as f32, values[2] as f32]
        }
        _ => return None,
    };
    // ARGB.as8BitChannel -> Mth.floor -> Java's saturating float-to-int cast.
    let [red, green, blue] = channels.map(|n| ((n * 255.0).floor() as i32) & 255);
    Some((255 << 24) | (red << 16) | (green << 8) | blue)
}
