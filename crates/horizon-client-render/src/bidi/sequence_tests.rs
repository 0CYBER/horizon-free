use super::*;

// Expected records were read from a real 26.3 client FormattedCharSink using
// ClientLanguage / FormattedBidiReorder and ICU 78.3, not this implementation.
#[test]
fn vanilla_sink_order_and_utf16_run_offsets() {
    for (input, expected, indices) in [
        ("שלום", "םולש", vec![3, 2, 1, 0]),
        (
            "abc אבג 123",
            "abc 123 גבא",
            vec![0, 1, 2, 3, 0, 1, 2, 3, 2, 1, 0],
        ),
        ("A😀B", "A😀B", vec![0, 1, 3]),
        ("سلام", "\u{fee1}\u{fefc}\u{feb3}", vec![2, 1, 0]),
    ] {
        let text = Text::literal(input);
        let output = visual_characters(&text);
        assert_eq!(output.iter().map(|v| v.0).collect::<String>(), expected);
        assert_eq!(output.iter().map(|v| v.2).collect::<Vec<_>>(), indices);
    }
}

#[test]
fn equal_styles_coalesce_but_different_styles_reset_sink_indices() {
    let bold = Style {
        bold: Some(true),
        ..Style::default()
    };
    let text = Text::from_spans(vec![
        Span::new("Hello, ", Style::default()),
        Span::new("Alex", bold.clone()),
        Span::new("!", Style::default()),
    ]);
    let output = visual_characters(&text);
    assert_eq!(
        output.iter().map(|v| v.2).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6, 0, 1, 2, 3, 0]
    );
    assert!(output[7..11].iter().all(|v| v.1 == &bold));
    let mixed = Text::from_spans(vec![
        Span::new("abc ", Style::default()),
        Span::new("אבג", bold.clone()),
        Span::new(" 123", Style::default()),
    ]);
    let output = visual_characters(&mixed);
    assert_eq!(
        output.iter().map(|v| v.2).collect::<Vec<_>>(),
        [0, 1, 2, 3, 0, 1, 2, 0, 2, 1, 0]
    );
    assert!(output[8..].iter().all(|v| v.1 == &bold));
    let text = Text::from_spans(vec![
        Span::new("ab", Style::default()),
        Span::new("cd", Style::default()),
    ]);
    assert_eq!(
        visual_characters(&text)
            .iter()
            .map(|v| v.2)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
}
