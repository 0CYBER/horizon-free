//! Invite codes: eight Crockford base32 symbols shown as `K7QM-3XPD`. The
//! first four are the nameplate the relay finds the host by; the last four
//! are the secret the key exchange uses, which only the players see.
use std::fmt;

/// Crockford base32: digits and letters without I, L, O and U.
pub const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Symbols in each half of a code.
pub const HALF: usize = 4;

/// The value of a symbol as a player may type it: any case, `O` for `0`,
/// `I` and `L` for `1`.
fn value(c: char) -> Option<u8> {
    let c = match c.to_ascii_uppercase() {
        'O' => '0',
        'I' | 'L' => '1',
        c => c,
    };
    ALPHABET
        .iter()
        .position(|&a| char::from(a) == c)
        .map(|i| i as u8)
}

/// Four symbols (20 bits) as their canonical text.
fn symbols(bits: u32) -> String {
    (0..HALF)
        .rev()
        .map(|i| char::from(ALPHABET[(bits >> (5 * i)) as usize & 31]))
        .collect()
}

/// A nameplate drawn from 20 random bits.
pub fn nameplate(bits: u32) -> String {
    symbols(bits & 0xF_FFFF)
}

/// A nameplate as a player or a client sent it, made canonical; None when it
/// isn't four symbols.
pub fn parse_nameplate(text: &str) -> Option<String> {
    let symbols: Option<Vec<u8>> = text.chars().map(value).collect();
    let symbols = symbols?;
    (symbols.len() == HALF).then(|| {
        symbols
            .iter()
            .map(|&v| char::from(ALPHABET[v as usize]))
            .collect()
    })
}

#[derive(Clone, PartialEq, Eq)]
pub struct Code {
    pub nameplate: String,
    pub secret: String,
}
impl Code {
    /// A code for a nameplate the relay handed out, with a fresh secret from
    /// the OS random generator.
    pub fn new(nameplate: &str) -> std::io::Result<Self> {
        let mut b = [0u8; 4];
        crate::fill_random(&mut b)?;
        Ok(Self {
            nameplate: nameplate.to_owned(),
            secret: symbols(u32::from_le_bytes(b) & 0xF_FFFF),
        })
    }
    /// A code as a player typed it: spaces, dashes and case don't matter, `O`
    /// reads as `0`, `I` and `L` as `1`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut out = String::with_capacity(2 * HALF);
        for c in text.chars() {
            if c.is_whitespace() || c == '-' {
                continue;
            }
            out.push(char::from(ALPHABET[value(c)? as usize]));
        }
        if out.len() != 2 * HALF {
            return None;
        }
        let (nameplate, secret) = out.split_at(HALF);
        Some(Self {
            nameplate: nameplate.to_owned(),
            secret: secret.to_owned(),
        })
    }
}
impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.nameplate, self.secret)
    }
}
/// The secret stays out of debug output and logs.
impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Code({}-****)", self.nameplate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_has_no_ambiguous_letters() {
        assert_eq!(ALPHABET.len(), 32);
        for c in *b"ILOU" {
            assert!(!ALPHABET.contains(&c));
        }
        let mut sorted = ALPHABET.to_vec();
        sorted.dedup();
        assert_eq!(sorted.len(), 32);
    }

    #[test]
    fn nameplates_cover_twenty_bits() {
        assert_eq!(nameplate(0), "0000");
        assert_eq!(nameplate(0xF_FFFF), "ZZZZ");
        assert_eq!(nameplate(0x8000 | 31), "100Z");
        // High bits beyond 20 are ignored.
        assert_eq!(nameplate(0xFFF0_0001), "0001");
        let mut seen = std::collections::HashSet::new();
        for bits in (0..0x10_0000).step_by(997) {
            assert!(seen.insert(nameplate(bits)));
        }
    }

    #[test]
    fn parses_what_players_type() {
        let code = Code::parse("k7qm-3xpd").unwrap();
        assert_eq!(code.nameplate, "K7QM");
        assert_eq!(code.secret, "3XPD");
        assert_eq!(code.to_string(), "K7QM-3XPD");
        assert_eq!(Code::parse(" K7QM 3XPD ").unwrap(), code);
        assert_eq!(Code::parse("K7QM3XPD").unwrap(), code);
        // O, I and L are the symbols they look like.
        let code = Code::parse("OIL0-1lio").unwrap();
        assert_eq!(code.to_string(), "0110-1110");
        assert!(Code::parse("K7QM-3XP").is_none());
        assert!(Code::parse("K7QM-3XPDA").is_none());
        assert!(Code::parse("K7QM-3XPU").is_none());
        assert!(Code::parse("").is_none());
    }

    #[test]
    fn fresh_secrets_are_canonical_and_differ() {
        let a = Code::new("K7QM").unwrap();
        assert_eq!(Code::parse(&a.to_string()).unwrap(), a);
        let different = (0..8).any(|_| Code::new("K7QM").unwrap().secret != a.secret);
        assert!(different);
        assert_eq!(format!("{a:?}"), "Code(K7QM-****)");
    }

    #[test]
    fn nameplates_parse() {
        assert_eq!(parse_nameplate("k7qm").as_deref(), Some("K7QM"));
        assert_eq!(parse_nameplate("K7Q"), None);
        assert_eq!(parse_nameplate("K7QU"), None);
    }
}
