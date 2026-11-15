use mod_api::{ClientHost, Text};

#[test]
#[ignore = "daily"]
fn visual_read_uses_live_translations_styles_and_bidi_outside_drawing() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let host: &dyn ClientHost = game;
        let mut argument = Text::literal("Alex");
        argument.style.bold = Some(true);
        argument.style.color = Some("red".into());
        let mut line = Text::translate("fixture.greet", vec![argument]);
        line.style.color = Some("gold".into());
        horizon_client::lang::scoped(
            horizon_client::lang::Language::of(
                "test",
                [("fixture.greet".into(), "Hello, %s!".into())],
            ),
            || {
                let output = host.text_visual_order(&line).unwrap();
                assert_eq!(
                    output
                        .iter()
                        .map(|v| char::from_u32(v.scalar).unwrap())
                        .collect::<String>(),
                    "Hello, Alex!"
                );
                assert_eq!(
                    output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
                    [0, 1, 2, 3, 4, 5, 6, 0, 1, 2, 3, 0]
                );
                assert!(
                    output[7..11].iter().all(|v| v.style.bold == Some(true)
                        && v.style.color.as_deref() == Some("#FF5555"))
                );
                assert!(
                    output[..7]
                        .iter()
                        .chain(&output[11..])
                        .all(|v| v.style.color.as_deref() == Some("#FFAA00"))
                );
            },
        );
        for (input, expected, indices) in [
            ("שלום", "םולש", vec![3, 2, 1, 0]),
            (
                "abc אבג 123",
                "abc 123 גבא",
                vec![0, 1, 2, 3, 0, 1, 2, 3, 2, 1, 0],
            ),
        ] {
            let output = host.text_visual_order(&Text::literal(input)).unwrap();
            assert_eq!(
                output
                    .iter()
                    .map(|v| char::from_u32(v.scalar).unwrap())
                    .collect::<String>(),
                expected
            );
            assert_eq!(
                output.iter().map(|v| v.source_index).collect::<Vec<_>>(),
                indices
            );
        }
        let mut styled = Text::literal("X");
        styled.style.color = Some("#123456".into());
        styled.style.bold = Some(false);
        styled.style.italic = Some(true);
        styled.style.underlined = Some(true);
        styled.style.strikethrough = Some(true);
        styled.style.obfuscated = Some(true);
        styled.style.shadow_color = Some(0x80123456);
        styled.style.font = Some("minecraft:uniform".into());
        styled.style.insertion = Some("insert".into());
        styled.style.click = Some(mod_api::text::TextClick::CopyToClipboard("copy".into()));
        styled.style.hover = Some(mod_api::TextHover::Text(Box::new(Text::literal("hover"))));
        let output = host.text_visual_order(&styled).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].style, styled.style);
        assert!(
            host.text_visual_order(&Text::literal(""))
                .unwrap()
                .is_empty()
        );
        let mut invalid = Text::literal("A");
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            let mut parent = Text::literal("");
            parent.children.push(invalid);
            invalid = parent;
        }
        assert_eq!(
            host.text_visual_order(&invalid),
            Err(mod_api::TextFormError::TooDeep)
        );
    });
}
