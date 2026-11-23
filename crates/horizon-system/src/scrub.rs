//! Takes out of text what could say who or where a player is, before it is kept to be sent:
//! the user folder's name in paths, the user name, e-mail and IP addresses, UUIDs (accounts),
//! and the terms the caller knows are private (world names, server addresses, account names).

/// What replaces each kind of match.
pub const USER: &str = "<user>";
pub const EMAIL: &str = "<email>";
pub const IP: &str = "<ip>";
pub const UUID: &str = "<uuid>";
pub const PRIVATE: &str = "<private>";

#[derive(Clone, Debug, Default)]
pub struct Scrubber {
    /// Exact terms to hide, longest first.
    terms: Vec<String>,
    user: Option<String>,
}

impl Scrubber {
    /// A scrubber that also knows this machine's user name.
    pub fn new() -> Self {
        let user = ["USER", "USERNAME", "LOGNAME"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|u| u.trim().len() >= 3));
        Self {
            terms: Vec::new(),
            user,
        }
    }

    /// Without the machine's user name (tests).
    pub fn bare() -> Self {
        Self::default()
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into()).filter(|u| u.trim().len() >= 3);
        self
    }

    /// Hides `term` wherever it appears. A term of 1 or 2 characters (the game accepts such
    /// player names) would blank out ordinary words, so it is hidden only as a whole word, in any
    /// case: not inside a longer run of letters, digits and `_`, the characters of a player name.
    pub fn term(mut self, term: impl Into<String>) -> Self {
        let term = term.into();
        if !term.trim().is_empty() && !self.terms.contains(&term) {
            self.terms.push(term);
            self.terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
        }
        self
    }

    pub fn scrub(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for term in &self.terms {
            out = if term.trim().len() >= 3 {
                out.replace(term.as_str(), PRIVATE)
            } else {
                replace_word(&out, term, PRIVATE)
            };
        }
        out = user_folders(&out);
        out = replace_tokens(&out, email_at);
        out = replace_tokens(&out, ipv4_at);
        out = replace_tokens(&out, ipv6_at);
        out = replace_tokens(&out, uuid_at);
        if let Some(user) = &self.user {
            out = replace_word(&out, user, USER);
        }
        out
    }
}

/// `/Users/ana/…`, `/home/ana/…`, `C:\Users\Ana Silva\…` (also JSON-escaped): the folder after
/// `Users` or `home` becomes [`USER`], the rest of the path stays. Windows names have spaces, so
/// the name runs to the next separator, quote, bracket or line end.
fn user_folders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for parent in ["Users", "home"] {
            for sep in ["/", "\\\\", "\\"] {
                let head = format!("{sep}{parent}{sep}");
                if let Some(after) = rest.strip_prefix(head.as_str()) {
                    let name_len = after
                        .find(|c: char| {
                            matches!(
                                c,
                                '/' | '\\' | '"' | '\'' | '\n' | '\r' | ')' | ']' | ',' | ';'
                            )
                        })
                        .unwrap_or(after.len());
                    out.push_str(&head);
                    if name_len > 0 {
                        out.push_str(USER);
                    }
                    rest = &after[name_len..];
                    continue 'scan;
                }
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Replaces every match of `matcher` (which returns the length of a match starting at a byte
/// offset, if one does) with its label.
fn replace_tokens(
    text: &str,
    matcher: fn(&[u8], usize) -> Option<(usize, &'static str)>,
) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if text.is_char_boundary(i)
            && let Some((len, label)) = matcher(bytes, i)
        {
            out.push_str(&text[copied..i]);
            out.push_str(label);
            i += len;
            copied = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether a token may start at `i`: not in the middle of a word, number or address.
fn starts_token(bytes: &[u8], i: usize) -> bool {
    i == 0 || !(word_byte(bytes[i - 1]) || matches!(bytes[i - 1], b'.' | b':' | b'-' | b'@'))
}

fn ends_token(bytes: &[u8], end: usize) -> bool {
    end == bytes.len() || !word_byte(bytes[end])
}

fn email_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    let local = |b: u8| word_byte(b) || matches!(b, b'.' | b'%' | b'+' | b'-');
    if !starts_token(bytes, i) || !local(bytes[i]) {
        return None;
    }
    let at = i + bytes[i..].iter().position(|&b| !local(b))?;
    if bytes.get(at) != Some(&b'@') {
        return None;
    }
    let domain = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-');
    let end = at + 1 + bytes[at + 1..].iter().take_while(|&&b| domain(b)).count();
    let host = &bytes[at + 1..end];
    let dot = host.iter().rposition(|&b| b == b'.')?;
    (dot > 0 && host.len() - dot > 2).then_some((end - i, EMAIL))
}

fn ipv4_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    if !starts_token(bytes, i) {
        return None;
    }
    let mut end = dotted_quad(bytes, i)?;
    // A port goes with it.
    if bytes.get(end) == Some(&b':') {
        let digits = bytes[end + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if (1..=5).contains(&digits) {
            end += 1 + digits;
        }
    }
    ends_token(bytes, end).then_some((end - i, IP))
}

/// Where four dotted decimal parts (each 0 to 255) starting at `i` end, if they do.
fn dotted_quad(bytes: &[u8], i: usize) -> Option<usize> {
    let mut end = i;
    for part in 0..4 {
        if part > 0 {
            if bytes.get(end) != Some(&b'.') {
                return None;
            }
            end += 1;
        }
        let digits = bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits == 0 || digits > 3 {
            return None;
        }
        let n: u32 = std::str::from_utf8(&bytes[end..end + digits])
            .ok()?
            .parse()
            .ok()?;
        if n > 255 {
            return None;
        }
        end += digits;
    }
    // A fifth part makes it a version number (`10.0.19045.1`), not an address.
    if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some(end)
}

/// Eight hex groups with colons, or fewer around a `::` (at least two, as in `fd00::1`), the last
/// 32 bits possibly written as an IPv4 address (`::ffff:192.168.1.10`), which goes with it, and
/// a trailing `::` too (`2a01:4f8:c0c:1::`). A digit in it is what leaves a Rust path such as
/// `a::b` alone; with only two groups and every colon in a `::` pair, as a path has them, the
/// digit has to be in the last group, so `f32::add` stays too. A bare `::1` (loopback) says
/// nothing about the player.
fn ipv6_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    if !starts_token(bytes, i) {
        return None;
    }
    let len = bytes[i..]
        .iter()
        .take_while(|b| b.is_ascii_hexdigit() || **b == b':')
        .count();
    let token = std::str::from_utf8(&bytes[i..i + len]).ok()?;
    // One colon after it ends the sentence (`2001:db8::1: refused`); a `::` is the address's.
    let trimmed = token.trim_end_matches(':');
    let mut token = if token.len() - trimmed.len() >= 2 {
        &token[..trimmed.len() + 2]
    } else {
        trimmed
    };
    let mut end = i + token.len();
    // An IPv4 address in the last two groups' place: the hex part is what comes before it.
    let mut quad = false;
    if bytes.get(end) == Some(&b'.')
        && let Some(colon) = token.rfind(':')
        && let Some(quad_end) = dotted_quad(bytes, i + colon + 1)
    {
        token = &token[..=colon];
        end = quad_end;
        quad = true;
    }
    let hex = if quad {
        &token[..token.len() - 1]
    } else {
        token
    };
    let groups: Vec<&str> = hex.split(':').collect();
    let filled = groups.iter().filter(|g| !g.is_empty()).count() + if quad { 2 } else { 0 };
    let digit = |g: &&str| g.bytes().any(|b| b.is_ascii_digit());
    // Every colon in a `::`, as in a code path (`a::b`): only a digit tells an address apart. A single colon
    // never stands in a path, so an address of letters only (`fdab:cdef::abcd`) is hidden too.
    let pathish = !token.replace("::", "").contains(':');
    let path_like = !quad && filled == 2 && pathish;
    let ok = groups.iter().all(|g| g.len() <= 4)
        && (filled == 8 || (token.contains("::") && filled >= 2))
        && (quad || !pathish || groups.iter().any(digit))
        && (!path_like
            || groups
                .iter()
                .rev()
                .find(|g| !g.is_empty())
                .is_some_and(digit));
    (ok && ends_token(bytes, end)).then_some((end - i, IP))
}

/// `0f8c…-…` (8-4-4-4-12) or the same 32 hex digits without dashes, as accounts are named.
fn uuid_at(bytes: &[u8], i: usize) -> Option<(usize, &'static str)> {
    if !starts_token(bytes, i) {
        return None;
    }
    let hex = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|s| s.iter().all(u8::is_ascii_hexdigit))
    };
    let dashed = [8usize, 13, 18, 23]
        .iter()
        .all(|&d| bytes.get(i + d) == Some(&b'-'))
        && [(0, 8), (9, 13), (14, 18), (19, 23), (24, 36)]
            .iter()
            .all(|&(a, b)| hex(i + a..i + b));
    if dashed && ends_token(bytes, i + 36) {
        return Some((36, UUID));
    }
    (hex(i..i + 32) && ends_token(bytes, i + 32)).then_some((32, UUID))
}

/// Whole-word occurrences of `word`, in any case.
fn replace_word(text: &str, word: &str, label: &str) -> String {
    // Compared on the text as it is, a character at a time: lowercasing the whole text would
    // change byte lengths outside ASCII (`İ` is 2 bytes and lowercases to 3), and its offsets
    // would no longer point into the text.
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut start = 0;
    while start < text.len() {
        let end = matches_at(text, start, word);
        let replaced = end.filter(|&end| {
            let alone = (start == 0 || !word_byte(bytes[start - 1])) && ends_token(bytes, end);
            // A label put in earlier (`<user>` for a user named "user") stays as it is.
            let label_inside =
                start > 0 && bytes[start - 1] == b'<' && bytes.get(end) == Some(&b'>');
            alone && !label_inside
        });
        if let Some(end) = replaced {
            out.push_str(&text[copied..start]);
            out.push_str(label);
            copied = end;
            start = end;
        } else {
            start += text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Where `word` ends when it starts at byte `at` of `text`, each character compared in any case.
fn matches_at(text: &str, at: usize, word: &str) -> Option<usize> {
    let mut hay = text[at..].chars();
    let mut end = at;
    for w in word.chars() {
        let h = hay.next()?;
        if h != w && !h.to_lowercase().eq(w.to_lowercase()) {
            return None;
        }
        end += h.len_utf8();
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lowercased haystack and the text must share their character boundaries, not just their
    /// length: `İ` grows by a byte and U+212A shrinks by two, so a text holding two `İ` and a
    /// U+212A kept its length while its offsets moved, and a later match sliced mid-character.
    #[test]
    fn a_text_whose_lowercase_moves_its_boundaries_is_matched_as_it_is() {
        let s = Scrubber::bare().with_user("i\u{307}");
        assert_eq!(s.scrub("\u{130}\u{130}\u{212A}"), "\u{130}\u{130}\u{212A}");
        // A name that is there, in another case, is still hidden.
        assert_eq!(
            s.scrub("\u{130}\u{130}\u{212A} ANA"),
            "\u{130}\u{130}\u{212A} ANA"
        );
        assert_eq!(
            Scrubber::bare().with_user("ana").scrub("aNa here"),
            "<user> here"
        );
    }

    /// Unicode elsewhere in the text, whose lowercase has another byte length, leaves the
    /// matching of a name in any case as it is.
    #[test]
    fn a_name_in_another_case_is_hidden_beside_unicode() {
        let s = Scrubber::bare().term("Zq").with_user("ana");
        assert_eq!(
            s.scrub("\u{130}stanbul \u{212A}: zQ quit, ZQ (Zqueue) ANA ok"),
            "\u{130}stanbul \u{212A}: <private> quit, <private> (Zqueue) <user> ok"
        );
    }

    #[test]
    fn user_folders_lose_their_name() {
        let s = Scrubber::bare();
        assert_eq!(
            s.scrub("at /Users/ana.b/code/horizon/src/main.rs:10"),
            "at /Users/<user>/code/horizon/src/main.rs:10"
        );
        assert_eq!(
            s.scrub("/home/ana/.local/share/horizon"),
            "/home/<user>/.local/share/horizon"
        );
        assert_eq!(
            s.scrub("C:\\Users\\Ana Silva\\x\nnext"),
            "C:\\Users\\<user>\\x\nnext"
        );
        assert_eq!(s.scrub(r"C:\Users\ana\AppData"), r"C:\Users\<user>\AppData");
        assert_eq!(
            s.scrub(r#"{"path":"C:\\Users\\ana\\AppData"}"#),
            r#"{"path":"C:\\Users\\<user>\\AppData"}"#
        );
        assert_eq!(s.scrub("/usr/home/"), "/usr/home/");
    }

    #[test]
    fn the_user_name_goes_wherever_it_stands_alone() {
        let s = Scrubber::bare().with_user("ludo");
        assert_eq!(
            s.scrub("Ludo's machine, user ludo"),
            "<user>'s machine, user <user>"
        );
        assert_eq!(s.scrub("ludovic"), "ludovic");
        assert_eq!(Scrubber::bare().with_user("al").scrub("al"), "al");
        assert_eq!(
            Scrubber::bare()
                .with_user("user")
                .scrub(r"C:\Users\user\AppData, user"),
            r"C:\Users\<user>\AppData, <user>"
        );
    }

    #[test]
    fn addresses_and_accounts_are_hidden() {
        let s = Scrubber::bare();
        assert_eq!(s.scrub("joined 192.168.1.20:25565 ok"), "joined <ip> ok");
        assert_eq!(s.scrub("from 10.0.0.1."), "from <ip>.");
        assert_eq!(s.scrub("[2a01:e0a:1f2::5]"), "[<ip>]");
        assert_eq!(
            s.scrub("joined [fd00::1]:25565 ok"),
            "joined [<ip>]:25565 ok"
        );
        assert_eq!(s.scrub("via fe80::1%eth0"), "via <ip>%eth0");
        assert_eq!(
            s.scrub("from ::ffff:192.168.1.10 and fd00::192.168.1.10."),
            "from <ip> and <ip>."
        );
        assert_eq!(
            s.scrub("net 2a01:4f8:c0c:1::/64 and fd00:1::, 2001:db8::1: refused"),
            "net <ip>/64 and <ip>, <ip>: refused"
        );
        assert_eq!(
            s.scrub("letters only: fdab:cdef::abcd, dead:beef:cafe:face:dead:beef:cafe:face"),
            "letters only: <ip>, <ip>"
        );
        assert_eq!(s.scrub("mail ana.b+mc@example.org now"), "mail <email> now");
        assert_eq!(
            s.scrub(
                "uuid 069a79f4-44e9-4726-a5be-fca90e38aaf5 and 069a79f444e94726a5befca90e38aaf5"
            ),
            "uuid <uuid> and <uuid>"
        );
    }

    #[test]
    fn code_and_versions_are_left_alone() {
        let s = Scrubber::bare();
        for text in [
            "std::panicking::begin_panic (src/game.rs:12:5)",
            "a::b::c",
            "f32::abs, f64::add(x), u8::MAX, d3d12::Device::new",
            "driver 31.0.15.3623, Windows 10.0.19045.1",
            "wgpu 30.0.1",
            "commit 9fceb02d0ae598e95dc970b74767f19372d61af8",
            "user@host",
        ] {
            assert_eq!(s.scrub(text), text);
        }
    }

    #[test]
    fn known_private_terms_go_first() {
        let s = Scrubber::bare()
            .term("My World")
            .term("play.example.net")
            .term("ab");
        assert_eq!(
            s.scrub("Saving My World; joined play.example.net:25565, ab"),
            "Saving <private>; joined <private>:25565, <private>"
        );
    }

    #[test]
    fn a_short_term_is_hidden_only_as_a_whole_word() {
        let s = Scrubber::bare().term("ab").term("Z");
        assert_eq!(
            s.scrub("about ab_c cab ab, (AB) z: Zed a_z 1ab ab1 ab"),
            "about ab_c cab <private>, (<private>) <private>: Zed a_z 1ab ab1 <private>"
        );
        // Longer terms still match inside words.
        assert_eq!(Scrubber::bare().term("abc").scrub("xabcx"), "x<private>x");
    }
}
