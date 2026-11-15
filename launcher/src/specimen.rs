//! The redesign's specimen: every motion the mockup asks of the launcher, drawn by the launcher
//! itself, for checking the effects in the real renderer before the pages take them
//! (`HORIZON_LAUNCHER_PAGE=specimen`; not a page players reach). Hover everything: the point of
//! this page is what happens under the pointer.
use crate::glass_ui::{self as ui};
use crate::theme::Theme;
use gpui::{div, prelude::*, px};
use horizon_theme::glass::Glass;
use std::sync::LazyLock;

/// Whether the launcher was opened on the specimen.
pub fn wanted() -> bool {
    static WANTED: LazyLock<bool> =
        LazyLock::new(|| std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("specimen"));
    *WANTED
}

#[derive(Clone, Copy, PartialEq)]
enum Seg {
    One,
    Two,
    Three,
}

/// The specimen's own state: which segmented option is chosen, so the pill can be made to slide.
static SEG: std::sync::Mutex<Seg> = std::sync::Mutex::new(Seg::One);

pub fn render(t: &Theme) -> gpui::Div {
    ui::set_on(true);
    ui::set_light(false);
    crate::glass::set_light(false);
    ui::set_look(Glass::default_look());
    crate::glass::set_look(Glass::default_look());
    let seg = *SEG.lock().unwrap();
    let pick = |v: Seg, window: &mut gpui::Window, _: &mut gpui::App| {
        *SEG.lock().unwrap() = v;
        window.refresh();
    };
    let cards = div()
        .flex()
        .items_start()
        .gap(px(16.))
        .child(ui::lift(
            &"spec-card-a".into(),
            ui::card("spec-card-a", 14., t)
                .w(px(220.))
                .p(px(16.))
                .gap(px(8.))
                .child(ui::text(16., gpui::FontWeight::SEMIBOLD, 1.).child("Lifting card"))
                .child(ui::text(13., gpui::FontWeight::NORMAL, 0.6).child(
                    "Rises 3 px on the spring while the pointer is on it; its outline warms \
                         to the accent and a glow opens under it.",
                )),
        ))
        .child(ui::lift(
            &"spec-card-b".into(),
            ui::card("spec-card-b", 14., t)
                .w(px(220.))
                .p(px(16.))
                .gap(px(8.))
                .child(ui::text(16., gpui::FontWeight::SEMIBOLD, 1.).child("Second card"))
                .child(
                    ui::text(13., gpui::FontWeight::NORMAL, 0.6)
                        .child("Pressing sinks it 1 px instead of scaling it."),
                ),
        ));
    let buttons = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(ui::button(
            "spec-primary".into(),
            "Primary, with sheen".into(),
            crate::Kind::Primary,
            t,
        ))
        .child(ui::button(
            "spec-secondary".into(),
            "Secondary".into(),
            crate::Kind::Secondary,
            t,
        ))
        .child(ui::button(
            "spec-ghost".into(),
            "Ghost".into(),
            crate::Kind::Ghost,
            t,
        ));
    let nav = div().flex().items_center().gap(px(2.)).child(ui::nav_tab(
        "spec-nav-open",
        "Open page",
        true,
        t,
    ));
    let nav2 = div().flex().items_center().gap(px(2.)).child(ui::nav_tab(
        "spec-nav-shut",
        "Another page",
        false,
        t,
    ));
    let segments = crate::segments(
        "spec-seg",
        &[(Seg::One, "One"), (Seg::Two, "Two"), (Seg::Three, "Three")],
        seg,
        t,
        pick,
    );
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .bg(ui::color(Glass::default_look().base))
        .flex()
        .flex_col()
        .items_start()
        .gap(px(28.))
        .p(px(40.))
        .child(ui::text(24., gpui::FontWeight::SEMIBOLD, 1.).child("Motion specimen"))
        .child(cards)
        .child(buttons)
        .child(nav)
        .child(nav2)
        .child(segments)
        .child(ui::text(13., gpui::FontWeight::NORMAL, 0.5).child(
            "Hover each part; click the segmented control to slide its pill. Reduced motion \
                 draws everything where it lands.",
        ))
}
