//! Command errors, completions and packrat integration for the shared SNBT parser.
use super::packrat::{Reason, State, Sugg};
use super::{CmdResult, CommandError, Reader, SuggestionsBuilder};
use horizon_core::nbt::{Compound, Tag, parse_snbt_argument, snbt_suggestions};

/// `TagParser.parseAsArgument`, adapting the canonical parser's relative cursor.
pub fn parse_tag_argument(reader: &mut Reader) -> CmdResult<Tag> {
    let start = reader.cursor;
    match parse_snbt_argument(reader.remaining()) {
        Ok((tag, used)) => {
            reader.cursor += used;
            Ok(tag)
        }
        Err(error) => {
            reader.cursor = start + error.cursor;
            let args: Vec<&dyn std::fmt::Display> = error
                .args
                .iter()
                .map(|arg| arg as &dyn std::fmt::Display)
                .collect();
            Err(CommandError::tr_at(error.key, &args, reader))
        }
    }
}
/// `TagParser.parseCompoundAsArgument`.
pub fn parse_compound_argument(reader: &mut Reader) -> CmdResult<Compound> {
    match parse_tag_argument(reader)? {
        Tag::Compound(c) => Ok(c),
        _ => Err(CommandError::tr_at(
            "argument.nbt.expected.compound",
            &[],
            reader,
        )),
    }
}
/// `TagParser.parseFully`.
pub fn parse_fully(text: &str) -> CmdResult<Tag> {
    let mut reader = Reader::new(text);
    let tag = parse_tag_argument(&mut reader)?;
    reader.skip_whitespace();
    if reader.can_read() {
        return Err(CommandError::tr_at("argument.nbt.trailing", &[], &reader));
    }
    Ok(tag)
}
/// `ParserBasedArgument.listSuggestions`, using the same grammar as decoding.
pub fn suggest(builder: &mut SuggestionsBuilder) {
    let (cursor, suggestions) = snbt_suggestions(builder.remaining());
    let mut offset = builder.offset(builder.start + cursor);
    offset.suggest_matching(suggestions.iter().map(String::as_str));
    builder.add(offset);
}
/// Parse with an outer state (TagParseRule inside another grammar).
pub fn rule_in<V: super::packrat::Value>(
    state: &mut State<V>,
    wrap: impl Fn(Tag) -> V,
) -> Option<V> {
    let mut r = state.reader();
    r.skip_whitespace();
    state.set_reader(&r);
    let mark = state.cursor;
    let mut reader = state.reader();
    match parse_tag_argument(&mut reader) {
        Ok(t) => {
            state.set_reader(&reader);
            Some(wrap(t))
        }
        Err(e) => {
            state.store(mark, Sugg::Empty, Reason::Error(e));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_fully;

    /// A crafted over-deep value must be rejected, never overflow the stack.
    #[test]
    fn deeply_nested_compound_is_rejected_not_overflowed() {
        let mut deep = String::from("1");
        for _ in 0..513 {
            deep = format!("{{a:{deep}}}");
        }
        assert!(parse_fully(&deep).is_err());
    }

    /// The `extra` chain of a JSON text component is the same SNBT shape.
    #[test]
    fn deeply_nested_list_is_rejected_not_overflowed() {
        let mut deep = String::from("\"x\"");
        for _ in 0..513 {
            deep = format!("[{deep}]");
        }
        assert!(parse_fully(&deep).is_err());
    }

    /// Shallow, real command values must still parse (the cap is not too low).
    #[test]
    fn shallow_values_still_parse() {
        assert!(parse_fully("{a:{b:{c:1}}}").is_ok());
        assert!(parse_fully("[[1,2],[3,4]]").is_ok());
    }

    /// Deep but real values parse: 50 nested compounds, and an item whose
    /// `container` holds an item with a `container`, ten levels down.
    #[test]
    fn deep_real_values_still_parse() {
        let mut compound = String::from("1");
        for _ in 0..50 {
            compound = format!("{{a:{compound}}}");
        }
        assert!(parse_fully(&compound).is_ok());
        let mut item = String::from("{id:\"minecraft:stone\",count:1}");
        for _ in 0..10 {
            item = format!(
                "{{id:\"minecraft:shulker_box\",count:1,components:{{\"minecraft:container\":[{{slot:0,item:{item}}}]}}}}"
            );
        }
        assert!(parse_fully(&item).is_ok());
    }
}

#[cfg(test)]
mod floating_zero_tests {
    use super::*;
    use horizon_core::nbt::{Limits, read_network};

    fn cases() -> Vec<(&'static str, Tag)> {
        vec![
            ("0.0f", Tag::Float(0)),
            ("-0.0f", Tag::Float(0)),
            ("0e0f", Tag::Float(0)),
            ("-0e0f", Tag::Float(0)),
            ("0.0d", Tag::Double(0)),
            ("-0.0d", Tag::Double(0)),
            ("0e0d", Tag::Double(0)),
            ("-0e0d", Tag::Double(0)),
            ("0.0", Tag::Double(0)),
            ("-0.0", Tag::Double(0)),
            ("0e0", Tag::Double(0)),
            ("-0e0", Tag::Double(0)),
            ("1.25f", Tag::Float(0x3fa0_0000)),
            ("-1.25f", Tag::Float(0xbfa0_0000)),
            ("1.25d", Tag::Double(0x3ff4_0000_0000_0000)),
            ("-1.25d", Tag::Double(0xbff4_0000_0000_0000)),
            ("1e-45f", Tag::Float(1)),
            ("-1e-45f", Tag::Float(0x8000_0001)),
            ("5e-324d", Tag::Double(1)),
            ("-5e-324d", Tag::Double(0x8000_0000_0000_0001)),
            ("-0", Tag::Int(0)),
        ]
    }
    #[test]
    fn command_text_keeps_tag_factory_zero_and_nonzero_bits() {
        for (text, expected) in cases() {
            assert_eq!(parse_fully(text).unwrap(), expected, "{text}");
        }
    }
    #[test]
    fn world_text_keeps_tag_factory_zero_and_nonzero_bits() {
        for (text, expected) in cases() {
            let (actual, used) = horizon_core::nbt::parse_snbt(text).unwrap();
            assert_eq!(used, text.len(), "{text}");
            assert_eq!(actual, expected, "{text}");
        }
    }
    #[test]
    fn binary_decode_already_canonicalizes_only_zero() {
        for (bits, expected) in [
            (0u32, 0),
            (0x8000_0000, 0),
            (1, 1),
            (0x8000_0001, 0x8000_0001),
            (0x3fa0_0000, 0x3fa0_0000),
            (0xbfa0_0000, 0xbfa0_0000),
        ] {
            let mut bytes = vec![5];
            bytes.extend(bits.to_be_bytes());
            assert_eq!(
                read_network(&bytes, Limits::default()).unwrap(),
                (Tag::Float(expected), bytes.len())
            );
        }
        for (bits, expected) in [
            (0u64, 0),
            (0x8000_0000_0000_0000, 0),
            (1, 1),
            (0x8000_0000_0000_0001, 0x8000_0000_0000_0001),
            (0x3ff4_0000_0000_0000, 0x3ff4_0000_0000_0000),
            (0xbff4_0000_0000_0000, 0xbff4_0000_0000_0000),
        ] {
            let mut bytes = vec![6];
            bytes.extend(bits.to_be_bytes());
            assert_eq!(
                read_network(&bytes, Limits::default()).unwrap(),
                (Tag::Double(expected), bytes.len())
            );
        }
    }
}

#[cfg(test)]
mod suggestion_tests {
    use super::*;

    fn suggestions(input: &str) -> Vec<String> {
        let mut builder = SuggestionsBuilder::new(input, 0);
        suggest(&mut builder);
        let mut texts: Vec<String> = builder.out.into_iter().map(|s| s.text).collect();
        texts.sort();
        texts
    }

    /// A failed parse offers what could continue it at the furthest position.
    #[test]
    fn failed_parses_offer_their_continuations() {
        assert_eq!(suggestions("{a"), [":"]);
        assert_eq!(suggestions("[1,"), ["]"]);
        assert_eq!(suggestions("{\"a"), ["\"", "'", "\\"]);
    }

    /// Candidates must match the text typed at that position: nothing continues `\q`.
    #[test]
    fn candidates_are_filtered_by_the_typed_text() {
        assert_eq!(suggestions("\"\\q"), Vec::<String>::new());
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;
    use std::fmt::Display;

    /// What `parse_fully` must report: the key, its arguments and the absolute cursor.
    fn assert_fails(text: &str, key: &str, args: &[&str], cursor: usize) {
        let args: Vec<&dyn Display> = args.iter().map(|arg| arg as &dyn Display).collect();
        let mut at = Reader::new(text);
        at.cursor = cursor;
        assert_eq!(
            parse_fully(text).unwrap_err(),
            CommandError::tr_at(key, &args, &at),
            "{text}"
        );
    }

    #[test]
    fn string_escape_errors_keep_their_key_and_cursor() {
        // An unknown escape fails as the first escape literal, at the letter.
        assert_fails("{v:'\\q'}", "argument.literal.incorrect", &["b"], 5);
        // A short or bad hex run is reported where the digits start.
        assert_fails("{v:'\\u12'}", "snbt.parser.expected_hex_escape", &["4"], 6);
        // A name the Unicode data lacks is reported after the closing brace; empty braces at it.
        assert_fails(
            "{v:'\\N{zzz}'}",
            "snbt.parser.invalid_character_name",
            &[],
            11,
        );
        assert_fails("{v:'\\N{}'}", "snbt.parser.invalid_character_name", &[], 7);
        // A string that never closes fails at the end of the input.
        assert_fails("{v:'abc", "snbt.parser.invalid_string_contents", &[], 7);
    }
}
