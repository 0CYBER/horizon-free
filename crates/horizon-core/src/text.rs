//! Java text conversions shared by registry and client resource binding.

/// JDK 25 Integer.parseInt decimal semantics, including BMP decimal digits.
pub fn parse_java_integer(text: &str) -> Option<i32> {
    let (negative, text) = if let Some(text) = text.strip_prefix('-') {
        (true, text)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if text.is_empty() {
        return None;
    }
    // BMP decimal digit runs accepted by Character.digit(char, 10) in JDK 25.
    const ZEROES: [u32; 37] = [
        0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
        0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
        0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
        0xff10,
    ];
    let mut value = 0i64;
    for c in text.chars() {
        let c = c as u32;
        let zero = ZEROES
            .iter()
            .copied()
            .find(|zero| c >= *zero && c < *zero + 10)?;
        value = value * 10 + i64::from(c - zero);
        if value > i64::from(i32::MAX) + i64::from(negative) {
            return None;
        }
    }
    Some(if negative { -value } else { value } as i32)
}
