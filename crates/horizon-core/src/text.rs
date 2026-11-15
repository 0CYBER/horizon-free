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
        let digit = java_digit(c, 10)?;
        value = value * 10 + i128::from(digit);
        if value > max + i128::from(negative) {
            return None;
        }
    }
    Some(if negative { -value } else { value })
}

/// `Character.digit(char, radix)`: the char overload sees no supplementary scalar.
fn java_digit(c: char, radix: u32) -> Option<u32> {
    java_character::digit(u16::try_from(u32::from(c)).ok()?, radix)
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
                .checked_add(u64::from(java_digit(digit, 16)?))?;
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
