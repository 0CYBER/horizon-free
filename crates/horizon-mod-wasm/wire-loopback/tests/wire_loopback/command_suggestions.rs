use super::*;

#[test]
fn command_suggestions_keep_optional_byte_offsets_and_tooltips() {
    // Independent bytes: strings have little-endian u32 byte lengths, options
    // have one-byte tags, and usize crosses as a little-endian u64 on both targets.
    let cases = [
        (
            mod_api::CommandSuggestion::new("stone"),
            &[5, 0, 0, 0, b's', b't', b'o', b'n', b'e', 0, 0][..],
        ),
        (
            mod_api::CommandSuggestion::with_tooltip("é", "tip").replacing_from(6),
            &[
                2, 0, 0, 0, 0xc3, 0xa9, 1, 3, 0, 0, 0, b't', b'i', b'p', 1, 6, 0, 0, 0, 0, 0, 0, 0,
            ][..],
        ),
    ];
    for (suggestion, expected) in cases {
        let mut output = Writer::new();
        output.put(&suggestion);
        assert_eq!(output.bytes(), expected);
        assert_eq!(
            Reader::decode::<mod_api::CommandSuggestion>(expected).unwrap(),
            suggestion
        );
    }
}
