use super::*;

#[test]
fn hovered_buttons_ask_for_the_hand_or_not_allowed() {
    use crate::widgets::Cursor;
    let mut f = Fixture::new();
    let atlas =
        crate::hud::HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/horizon-test-assets")).to_owned(),
        ))
        .unwrap();
    let cursor_over = |title: &Title, label: Option<&str>| {
        let mouse = label.map_or((0.0, 0.0), |label| {
            let buttons = title.buttons(480.0, 262.0);
            let b = buttons.iter().find(|b| b.label == label).unwrap();
            (b.x + b.w / 2.0, b.y + 10.0)
        });
        let _ = crate::widgets::take_cursor();
        title.draw(
            &atlas,
            &mut HudBatch::scratch([480, 262]),
            mouse,
            true,
            &mut |_, _| {},
        );
        crate::widgets::take_cursor()
    };
    assert_eq!(
        cursor_over(&f.title, Some("Singleplayer")),
        Cursor::PointingHand
    );
    assert_eq!(cursor_over(&f.title, None), Cursor::Arrow);
    // The world list's footer is inactive until a world is selected.
    f.world("A", "Alpha");
    f.title.enter_singleplayer();
    assert_eq!(cursor_over(&f.title, Some("Edit")), Cursor::NotAllowed);
}
