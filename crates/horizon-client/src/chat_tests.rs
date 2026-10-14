use super::*;
use crate::{font::Font, hud::Rect};
use std::{collections::HashMap, path::Path};

fn font() -> Font {
    // The missing-glyph metrics let these tests run without game assets.
    Font::load(
        Path::new("/nonexistent-horizon-chat-test-resources"),
        &HashMap::new(),
        &[],
        (1, 1),
        (0, 0),
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        },
    )
}

fn add(chat: &mut Chat, font: &Font, content: Text) {
    for content in font.wrap_components(&content, chat.max_width()) {
        chat.lines.insert(
            0,
            Line {
                added: 0,
                content,
                indicator: None,
            },
        );
    }
}

#[test]
fn clicking_seed_copies_the_raw_value_only_inside_the_link() {
    let font = font();
    for seed in [1, -42, i64::MIN, i64::MAX] {
        let mut chat = Chat::default();
        let seed_text = Text::copyable(seed.to_string()).with_color(text::GOLD);
        assert_eq!(seed_text.plain(), format!("[{seed}]"));
        assert_eq!(seed_text.spans[0].style.color, Some(text::GOLD));
        assert_eq!(seed_text.spans[1].style.color, Some(text::GREEN));
        assert_eq!(seed_text.spans[2].style.color, Some(text::GOLD));
        add(&mut chat, &font, Text::literal("Seed: ").append(seed_text));
        let expected = Some(ClickEvent::CopyToClipboard(seed.to_string()));
        let start = 4.0 + f64::from(font.width("Seed: ["));
        let end = start + f64::from(font.width(&seed.to_string()));
        assert_eq!(chat.click_at(&font, 360, start, 315.0), expected);
        assert_eq!(chat.click_at(&font, 360, end - 0.5, 315.0), expected);
        for (x, y) in [
            (start - 0.5, 315.0),
            (end, 315.0),
            (3.0, 315.0),
            (start, 320.0),
            (start, 310.0),
        ] {
            assert_eq!(chat.click_at(&font, 360, x, y), None, "{x}, {y}");
        }
    }
}

#[test]
fn click_targets_match_rendered_spans_when_scaled_wrapped_and_scrolled() {
    let font = font();
    let screenshot = ClickEvent::OpenFile("/tmp/My screenshots/世界.png".into());
    for scale in [0.5, 0.75, 1.0] {
        for line_spacing in [0.0, 0.5, 1.0] {
            let mut chat = Chat::default();
            chat.options.scale = scale;
            chat.options.width = 0.0;
            chat.options.line_spacing = line_spacing;
            for _ in 0..20 {
                add(
                    &mut chat,
                    &font,
                    Text::literal("Saved screenshot as ").append(Text::styled(
                        "2026-10-14_18.55.14.png",
                        Style {
                            click: Some(screenshot.clone()),
                            bold: Some(true),
                            underlined: Some(true),
                            ..Style::default()
                        },
                    )),
                );
            }
            for scroll in [0, 3, 12] {
                chat.scroll = scroll;
                let mut links = 0;
                for draw in chat.layout(361, 0, true) {
                    if let GuiDraw::Text { text, x, y, .. } = draw {
                        let mut at = x as f64;
                        for (c, style) in text.chars() {
                            let width = f64::from(font.advance(c, style));
                            let cursor_x = (4.0 + at + width / 2.0) * scale;
                            let cursor_y = (y as f64 + 3.0) * scale;
                            assert_eq!(
                                chat.click_at(&font, 361, cursor_x, cursor_y),
                                style.click,
                                "scale={scale}, spacing={line_spacing}, scroll={scroll}, char={c}"
                            );
                            links += usize::from(style.click.is_some());
                            at += width;
                        }
                    }
                }
                assert!(links > 0);
                // The history above the focused viewport is not clickable.
                let top = ((361 - 40) as f64 / scale).floor()
                    - chat.lines_per_page(true) as f64 * chat.options.line_height() as f64;
                assert_eq!(
                    chat.click_at(&font, 361, 10.0 * scale, (top - 1.0) * scale),
                    None
                );
            }
        }
    }
}

#[test]
fn locate_coordinates_fill_chat_with_teleport_without_submitting() {
    let font = font();
    let m = Measure(&font);
    let registries = Registries::default();
    let commands = Commands {
        dispatcher: crate::command::vanilla::game_dispatcher(),
        registries: &registries,
    };
    // Structures and POIs use relative Y; biome results include absolute Y.
    for (y, expected) in [
        (None, "/tp @s -1136 ~ 848"),
        (Some(73), "/tp @s -1136 73 848"),
    ] {
        let link = Text::teleport_link(-1136, y, 848);
        assert_eq!(link.component()["click_event"]["action"], "suggest_command");
        assert_eq!(link.component()["click_event"]["command"], expected);
        let mut chat = Chat::default();
        add(&mut chat, &font, Text::literal("Found at ").append(link));
        let mut screen = ChatScreen::new(&m, &commands, "draft", 0, 640, 360);
        let x = 4.0 + f64::from(font.width("Found at "));
        assert!(!screen.click(&m, &commands, x, 315.0));
        let Some(ClickEvent::SuggestCommand(command)) = chat.click_at(&font, 360, x, 315.0) else {
            panic!("locate coordinates must suggest a command");
        };
        screen.suggest_command(&m, &commands, &command);
        assert_eq!(screen.value(), expected);
        assert_eq!(screen.input.cursor, expected.chars().count());
        assert_eq!(screen.input.highlight, screen.input.cursor);
        // Submission is a separate, explicit Enter press.
        assert_eq!(
            screen.key(
                &m,
                &commands,
                &[],
                winit::keyboard::KeyCode::Enter,
                false,
                false
            ),
            ChatAction::Submit(expected.to_owned())
        );
    }
}
