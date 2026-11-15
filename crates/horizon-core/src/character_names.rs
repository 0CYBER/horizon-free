//! Java 25 character names for SNBT named escapes, using Unicode 16.0.
//!
//! Explicit names come from `unicode_names2` 2.0.0. Range bounds are from
//! https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt; Java's block-name
//! spelling and control names follow the public `Character` API and a JVM oracle.

/// `Character.codePointOf`: trim Java ASCII whitespace and match names without
/// Unicode's permissive space/hyphen folding or name aliases. Returns `u32` so
/// named UTF-16 surrogates remain representable.
pub fn code_point_of(name: &str) -> Option<u32> {
    let name = name.trim_matches(|c: char| c <= '\u{20}');
    if !name.is_ascii() || name.is_empty() {
        return None;
    }
    for (index, canonical) in CONTROLS.iter().enumerate() {
        if name.eq_ignore_ascii_case(canonical) {
            return Some(if index < 32 {
                index as u32
            } else {
                index as u32 + 95
            });
        }
    }
    if let Some((prefix, hex)) = name.rsplit_once(' ')
        && !hex.is_empty()
        && !hex.starts_with('0')
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
        && let Ok(cp) = u32::from_str_radix(hex, 16)
        && let Some(block) = range_name(cp)
        && prefix.eq_ignore_ascii_case(block)
    {
        return Some(cp);
    }
    let character = unicode_names2::character(name)?;
    let cp = character as u32;
    // Unicode uses algorithmic ideograph/Hangul names, unlike Java. Control
    // aliases also differ, and must only be resolved by the table above.
    if range_name(cp).is_some() || character.is_control() {
        return None;
    }
    let canonical = unicode_names2::name(character)?;
    canonical
        .flat_map(str::bytes)
        .eq(name.bytes().map(|b| b.to_ascii_uppercase()))
        .then_some(cp)
}

fn range_name(cp: u32) -> Option<&'static str> {
    Some(match cp {
        0x3400..=0x4DBF => "CJK UNIFIED IDEOGRAPHS EXTENSION A",
        0x4E00..=0x9FFF => "CJK UNIFIED IDEOGRAPHS",
        0xAC00..=0xD7A3 => "HANGUL SYLLABLES",
        0xD800..=0xDB7F => "HIGH SURROGATES",
        0xDB80..=0xDBFF => "HIGH PRIVATE USE SURROGATES",
        0xDC00..=0xDFFF => "LOW SURROGATES",
        0xE000..=0xF8FF => "PRIVATE USE AREA",
        0x17000..=0x187F7 => "TANGUT",
        0x18D00..=0x18D08 => "TANGUT SUPPLEMENT",
        0x20000..=0x2A6DF => "CJK UNIFIED IDEOGRAPHS EXTENSION B",
        0x2A700..=0x2B739 => "CJK UNIFIED IDEOGRAPHS EXTENSION C",
        0x2B740..=0x2B81D => "CJK UNIFIED IDEOGRAPHS EXTENSION D",
        0x2B820..=0x2CEA1 => "CJK UNIFIED IDEOGRAPHS EXTENSION E",
        0x2CEB0..=0x2EBE0 => "CJK UNIFIED IDEOGRAPHS EXTENSION F",
        0x2EBF0..=0x2EE5D => "CJK UNIFIED IDEOGRAPHS EXTENSION I",
        0x30000..=0x3134A => "CJK UNIFIED IDEOGRAPHS EXTENSION G",
        0x31350..=0x323AF => "CJK UNIFIED IDEOGRAPHS EXTENSION H",
        0xF0000..=0xFFFFD => "SUPPLEMENTARY PRIVATE USE AREA A",
        0x100000..=0x10FFFD => "SUPPLEMENTARY PRIVATE USE AREA B",
        _ => return None,
    })
}

// Java uses UnicodeData's historical control names, not Unicode name aliases.
// U+0084 has no historical name and receives Java's block-plus-hex fallback.
const CONTROLS: [&str; 65] = [
    "NULL",
    "START OF HEADING",
    "START OF TEXT",
    "END OF TEXT",
    "END OF TRANSMISSION",
    "ENQUIRY",
    "ACKNOWLEDGE",
    "BEL",
    "BACKSPACE",
    "CHARACTER TABULATION",
    "LINE FEED (LF)",
    "LINE TABULATION",
    "FORM FEED (FF)",
    "CARRIAGE RETURN (CR)",
    "SHIFT OUT",
    "SHIFT IN",
    "DATA LINK ESCAPE",
    "DEVICE CONTROL ONE",
    "DEVICE CONTROL TWO",
    "DEVICE CONTROL THREE",
    "DEVICE CONTROL FOUR",
    "NEGATIVE ACKNOWLEDGE",
    "SYNCHRONOUS IDLE",
    "END OF TRANSMISSION BLOCK",
    "CANCEL",
    "END OF MEDIUM",
    "SUBSTITUTE",
    "ESCAPE",
    "INFORMATION SEPARATOR FOUR",
    "INFORMATION SEPARATOR THREE",
    "INFORMATION SEPARATOR TWO",
    "INFORMATION SEPARATOR ONE",
    "DELETE",
    "PADDING CHARACTER",
    "HIGH OCTET PRESET",
    "BREAK PERMITTED HERE",
    "NO BREAK HERE",
    "LATIN 1 SUPPLEMENT 84",
    "NEXT LINE (NEL)",
    "START OF SELECTED AREA",
    "END OF SELECTED AREA",
    "CHARACTER TABULATION SET",
    "CHARACTER TABULATION WITH JUSTIFICATION",
    "LINE TABULATION SET",
    "PARTIAL LINE FORWARD",
    "PARTIAL LINE BACKWARD",
    "REVERSE LINE FEED",
    "SINGLE SHIFT TWO",
    "SINGLE SHIFT THREE",
    "DEVICE CONTROL STRING",
    "PRIVATE USE ONE",
    "PRIVATE USE TWO",
    "SET TRANSMIT STATE",
    "CANCEL CHARACTER",
    "MESSAGE WAITING",
    "START OF GUARDED AREA",
    "END OF GUARDED AREA",
    "START OF STRING",
    "SINGLE GRAPHIC CHARACTER INTRODUCER",
    "SINGLE CHARACTER INTRODUCER",
    "CONTROL SEQUENCE INTRODUCER",
    "STRING TERMINATOR",
    "OPERATING SYSTEM COMMAND",
    "PRIVACY MESSAGE",
    "APPLICATION PROGRAM COMMAND",
];

#[cfg(test)]
mod tests {
    use super::{CONTROLS, code_point_of, range_name};

    #[test]
    fn strict_names_and_java_controls() {
        for (name, cp) in [
            ("\0 snowman\t", 0x2603),
            ("BEL", 7),
            ("PADDING CHARACTER", 0x80),
            ("HIGH OCTET PRESET", 0x81),
            ("LATIN 1 SUPPLEMENT 84", 0x84),
            ("SINGLE GRAPHIC CHARACTER INTRODUCER", 0x99),
            ("NUSHU CHARACTER-1B170", 0x1B170),
            ("KHITAN SMALL SCRIPT CHARACTER-18B00", 0x18B00),
        ] {
            assert_eq!(code_point_of(name), Some(cp), "{name}");
        }
        for (index, name) in CONTROLS.iter().enumerate() {
            let cp = if index < 32 { index } else { index + 95 } as u32;
            assert_eq!(code_point_of(name), Some(cp));
        }
        for name in [
            "s n o w m a n",
            "LOW-LINE",
            "LOW_LINE",
            "NUL",
            "LF",
            "LINE FEED",
            "CJK UNIFIED IDEOGRAPH-4E00",
            "HANGUL SYLLABLE GA",
            "BASIC LATIN 41",
            "\u{a0}SNOWMAN",
            "TANGUT 017000",
            "TANGUT 187F8",
            "TANGUT SUPPLEMENT 18D09",
            "CJK UNIFIED IDEOGRAPHS EXTENSION H 323B0",
            "SUPPLEMENTARY PRIVATE USE AREA B 10FFFF",
            "CJK UNIFIED IDEOGRAPHS EXTENSION J 323B0",
        ] {
            assert_eq!(code_point_of(name), None, "{name}");
        }
    }

    #[test]
    fn unicode_16_range_names_include_surrogates() {
        for cp in [
            0x3400, 0x4DBF, 0x4E00, 0x9FFF, 0xAC00, 0xD7A3, 0xD800, 0xDB7F, 0xDB80, 0xDBFF, 0xDC00,
            0xDFFF, 0xE000, 0xF8FF, 0x17000, 0x187F7, 0x18D00, 0x18D08, 0x20000, 0x2A6DF, 0x2A700,
            0x2B739, 0x2B740, 0x2B81D, 0x2B820, 0x2CEA1, 0x2CEB0, 0x2EBE0, 0x2EBF0, 0x2EE5D,
            0x30000, 0x3134A, 0x31350, 0x323AF, 0xF0000, 0xFFFFD, 0x100000, 0x10FFFD,
        ] {
            let name = format!("{} {cp:X}", range_name(cp).unwrap());
            assert_eq!(code_point_of(&name), Some(cp), "{name}");
        }
        for cp in [
            0x378, 0xD7A4, 0x187F8, 0x18D09, 0x2EE5E, 0x323B0, 0xFFFFE, 0x10FFFF,
        ] {
            assert_eq!(range_name(cp), None);
        }
    }
}
