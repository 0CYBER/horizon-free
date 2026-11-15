//! Java's floating-point spelling, shared by SNBT and JSON numeric conversions.
//!
//! Decimal selection follows the public Java 25 `Double.toString` and
//! `Float.toString` contracts: choose the closest shortest round-trip decimal
//! (considering both one and two digits when one suffices), breaking ties with
//! an even significand. No Java runtime implementation source is used.

use std::fmt::{self, Write};

// Scientific f64 text needs at most 24 ASCII bytes, including a sign.
struct NumberBuffer {
    bytes: [u8; 32],
    len: usize,
}

impl NumberBuffer {
    fn new() -> Self {
        Self {
            bytes: [0; 32],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("numeric formatting is ASCII")
    }
}

impl Write for NumberBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        let destination = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn render_nearest(
    negative: bool,
    value: f64,
    shortest: &str,
    round_trips: impl FnOnce(&str) -> bool,
) -> String {
    let mantissa = shortest.split_once('e').expect("scientific notation").0;
    let length = mantissa.bytes().filter(u8::is_ascii_digit).count();
    let precision = length.max(2) - 1;
    let mut nearest = NumberBuffer::new();
    // Explicit precision rounds the exact binary value, with ties to even;
    // Rust's shortest formatting can instead choose the odd upper candidate.
    write!(&mut nearest, "{value:.precision$e}").expect("scientific text fits");
    let candidate = nearest.as_str();
    // At normal powers of two the round-trip interval can be asymmetric.
    // Elsewhere it is symmetric, so the closest decimal must round back if
    // any decimal on this grid does; no parse is needed for those values.
    // A float widened to f64 retains whether it is a power of two.
    let power_of_two = value.to_bits() & 0x000f_ffff_ffff_ffff == 0;
    let selected = if !power_of_two || round_trips(candidate) {
        candidate
    } else {
        // The nearest grid point is outside the asymmetric interval; the
        // original shortest decimal is the only grid point inside it.
        shortest
    };
    let (mantissa, exponent) = selected.split_once('e').expect("scientific notation");
    let mut digits = [0; 17];
    let mut len = 0;
    for digit in mantissa.bytes().filter(u8::is_ascii_digit) {
        digits[len] = digit;
        len += 1;
    }
    while len > 1 && digits[len - 1] == b'0' {
        len -= 1;
    }
    render(
        negative,
        std::str::from_utf8(&digits[..len]).expect("decimal digits are ASCII"),
        exponent.parse().expect("scientific exponent"),
    )
}
/// Digits and decimal exponent of the shortest round-trip form.
fn shortest(text: &str) -> (String, i32) {
    // `{:e}` yields "d.ddde-5" or "de5".
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    (digits, exponent.parse().unwrap_or(0))
}

fn render(negative: bool, digits: &str, exponent: i32) -> String {
    let mut out = String::with_capacity(24);
    if negative {
        out.push('-');
    }
    let digits = if digits.is_empty() { "0" } else { digits };
    if (-3..7).contains(&exponent) {
        if exponent >= 0 {
            let int_len = exponent as usize + 1;
            if digits.len() <= int_len {
                out.push_str(digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exponent - 1) as usize));
            out.push_str(digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        write!(&mut out, "{exponent}").expect("writing to a string");
    }
    out
}

/// `Double.toString`.
pub fn double_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    let magnitude = value.abs();
    let mut shortest = NumberBuffer::new();
    write!(&mut shortest, "{magnitude:e}").expect("scientific text fits");
    render_nearest(
        value.is_sign_negative(),
        magnitude,
        shortest.as_str(),
        |text| text.parse::<f64>().is_ok_and(|parsed| parsed == magnitude),
    )
}

/// `Float.toString`.
pub fn float_to_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    let magnitude = value.abs();
    let mut shortest = NumberBuffer::new();
    write!(&mut shortest, "{magnitude:e}").expect("scientific text fits");
    render_nearest(
        value.is_sign_negative(),
        f64::from(magnitude),
        shortest.as_str(),
        |text| text.parse::<f32>().is_ok_and(|parsed| parsed == magnitude),
    )
}
/// `String.format(Locale.ROOT, "%.<n>f", value)`: Java rounds the shortest
/// round-trip decimal expansion half-up (so 1.005 gives 1.01).
pub fn format_fixed(value: f64, decimals: usize) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    let negative = value.is_sign_negative();
    let (digits, exponent) = if value == 0.0 {
        ("0".to_owned(), 0)
    } else {
        shortest(&format!("{:e}", value.abs()))
    };
    let mut int_part: Vec<u8> = Vec::new();
    let mut frac: Vec<u8> = Vec::new();
    let d: Vec<u8> = digits.bytes().map(|b| b - b'0').collect();
    let point = exponent + 1; // digits before the decimal point
    if point <= 0 {
        int_part.push(0);
        frac.extend(std::iter::repeat_n(0, (-point) as usize));
        frac.extend(&d);
    } else {
        let point = point as usize;
        for i in 0..point {
            int_part.push(d.get(i).copied().unwrap_or(0));
        }
        if d.len() > point {
            frac.extend(&d[point..]);
        }
    }
    let round_up = frac.get(decimals).is_some_and(|&x| x >= 5);
    frac.truncate(decimals);
    frac.resize(decimals, 0);
    if round_up {
        let mut carry = true;
        for x in frac.iter_mut().rev().chain(int_part.iter_mut().rev()) {
            if !carry {
                break;
            }
            if *x == 9 {
                *x = 0;
            } else {
                *x += 1;
                carry = false;
            }
        }
        if carry {
            int_part.insert(0, 1);
        }
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.extend(int_part.iter().map(|&x| (b'0' + x) as char));
    if decimals > 0 {
        out.push('.');
        out.extend(frac.iter().map(|&x| (b'0' + x) as char));
    }
    out
}

/// `%.2f`.
pub fn format_fixed2(value: f64) -> String {
    format_fixed(value, 2)
}

#[cfg(test)]
mod moved_tests {
    use super::*;
    #[test]
    fn java_number_text() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(0.5), "0.5");
        assert_eq!(double_to_string(0.0001), "1.0E-4");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(1234567.0), "1234567.0");
        assert_eq!(double_to_string(12345678.0), "1.2345678E7");
        assert_eq!(double_to_string(100.0), "100.0");
        assert_eq!(float_to_string(0.15), "0.15");
        assert_eq!(float_to_string(1.0e10), "1.0E10");
        assert_eq!(double_to_string(-2.5), "-2.5");
        for (bits, expected) in [
            (0x0000_0001, "1.4E-45"),
            (0x0000_0002, "2.8E-45"),
            (0x0000_0007, "9.8E-45"),
            (0x007f_ffff, "1.1754942E-38"),
            (0x0080_0000, "1.1754944E-38"),
            (0x0080_0001, "1.1754945E-38"),
            (0x3980_0000, "2.4414062E-4"),
            (0x44a4_5900, "1314.7812"),
            (0x4811_6728, "148892.62"),
            (0x4a00_0001, "2097152.2"),
            (0x7f7f_ffff, "3.4028235E38"),
        ] {
            assert_eq!(float_to_string(f32::from_bits(bits)), expected);
            assert_eq!(
                float_to_string(f32::from_bits(bits | 0x8000_0000)),
                format!("-{expected}")
            );
        }
        for (bits, expected) in [
            (0x0000_0000_0000_0001, "4.9E-324"),
            (0x0000_0000_0000_0002, "9.9E-324"),
            (0x0000_0000_0000_0014, "9.9E-323"),
            (0x000f_ffff_ffff_ffff, "2.225073858507201E-308"),
            (0x0010_0000_0000_0000, "2.2250738585072014E-308"),
            (0x0010_0000_0000_0001, "2.225073858507202E-308"),
            (0x3fef_ffff_ffff_ffff, "0.9999999999999999"),
            (0x3ff0_0000_0000_0001, "1.0000000000000002"),
            (0x3e60_0000_0000_0000, "2.9802322387695312E-8"),
            (0x4300_0ef8_712f_e8d2, "5.650074801267462E14"),
            (0x4310_0000_0000_0001, "1.1258999068426242E15"),
            (0x7fef_ffff_ffff_ffff, "1.7976931348623157E308"),
        ] {
            assert_eq!(double_to_string(f64::from_bits(bits)), expected);
            assert_eq!(
                double_to_string(f64::from_bits(bits | 0x8000_0000_0000_0000)),
                format!("-{expected}")
            );
        }
        assert_eq!(float_to_string(-0.0), "-0.0");
        assert_eq!(double_to_string(-0.0), "-0.0");
        assert_eq!(float_to_string(f32::INFINITY), "Infinity");
        assert_eq!(double_to_string(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(float_to_string(f32::NAN), "NaN");
        assert_eq!(double_to_string(f64::NAN), "NaN");
    }
}
