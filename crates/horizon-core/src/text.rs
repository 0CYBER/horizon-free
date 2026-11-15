//! Java text conversions shared by registry and client resource binding.

use crate::java_character;

/// JDK 25 Integer.parseInt decimal semantics, including BMP decimal digits.
pub fn parse_java_integer(text: &str) -> Option<i32> {
    parse_java_decimal(text, i128::from(i32::MAX)).map(|value| value as i32)
}

/// JDK 25 Long.parseLong decimal semantics, including BMP decimal digits.
pub fn parse_java_long(text: &str) -> Option<i64> {
    parse_java_decimal(text, i128::from(i64::MAX)).map(|value| value as i64)
}
// BMP decimal digit runs accepted by Character.digit(char, 10) in JDK 25.
const ZEROES: [u32; 37] = [
    0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
    0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
    0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0, 0xff10,
];

fn parse_java_decimal(text: &str, max: i128) -> Option<i128> {
    let (negative, text) = if let Some(text) = text.strip_prefix('-') {
        (true, text)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if text.is_empty() {
        return None;
    }
    let mut value = 0i128;
    for c in text.chars() {
        let digit = u16::try_from(u32::from(c))
            .ok()
            .and_then(|unit| java_character::digit(unit, 10))?;
        value = value * 10 + i128::from(digit);
        if value > max + i128::from(negative) {
            return None;
        }
    }
    Some(if negative { -value } else { value })
}

/// `Character.digit(char, 16)`, restricted to the BMP like Java's char overload.
fn java_hex_digit(c: char) -> Option<u32> {
    let c = c as u32;
    match c {
        0x41..=0x46 => Some(c - 0x41 + 10),
        0x61..=0x66 => Some(c - 0x61 + 10),
        0xff21..=0xff26 => Some(c - 0xff21 + 10),
        0xff41..=0xff46 => Some(c - 0xff41 + 10),
        _ => ZEROES
            .iter()
            .copied()
            .find(|zero| c >= *zero && c < *zero + 10)
            .map(|zero| c - zero),
    }
}

/// `UUID.fromString`: signed-long hex groups, masked to UUID field widths.
pub fn parse_java_uuid(name: &str) -> Option<u128> {
    if name.encode_utf16().count() > 36 {
        return None;
    }
    let mut parts = name.split('-');
    let mut values = [0u64; 5];
    for value in &mut values {
        let group = parts.next()?;
        let digits = group.strip_prefix('+').unwrap_or(group);
        if digits.is_empty() {
            return None;
        }
        for digit in digits.chars() {
            *value = value
                .checked_mul(16)?
                .checked_add(u64::from(java_hex_digit(digit)?))?;
            if *value > i64::MAX as u64 {
                return None;
            }
        }
    }
    if parts.next().is_some() {
        return None;
    }
    let most =
        ((values[0] & 0xffff_ffff) << 32) | ((values[1] & 0xffff) << 16) | (values[2] & 0xffff);
    let least = ((values[3] & 0xffff) << 48) | (values[4] & 0xffff_ffff_ffff);
    Some((u128::from(most) << 64) | u128::from(least))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_integer_and_long_boundaries_follow_java() {
        for (text, expected) in [
            ("2147483647", Some(i32::MAX)),
            ("-2147483648", Some(i32::MIN)),
            ("2147483648", None),
            ("-2147483649", None),
            ("+\u{0661}\u{ff12}", Some(12)),
        ] {
            assert_eq!(parse_java_integer(text), expected, "{text}");
        }
        for (text, expected) in [
            ("9223372036854775807", Some(i64::MAX)),
            ("-9223372036854775808", Some(i64::MIN)),
            ("9223372036854775808", None),
            ("-9223372036854775809", None),
            ("+\u{0661}\u{ff12}", Some(12)),
        ] {
            assert_eq!(parse_java_long(text), expected, "{text}");
        }
        for text in [
            "",
            "+",
            "-",
            " 12",
            "12 ",
            "1_2",
            "\u{b2}",
            "\u{2460}",
            "\u{1d7cf}",
            "\u{2212}1",
        ] {
            assert_eq!(parse_java_integer(text), None, "{text}");
            assert_eq!(parse_java_long(text), None, "{text}");
        }
        let zeroes = format!("{}12", "0".repeat(100));
        assert_eq!(parse_java_integer(&zeroes), Some(12));
        assert_eq!(parse_java_long(&zeroes), Some(12));
        assert_eq!(parse_java_long(&"9".repeat(100)), None);
    }
}
