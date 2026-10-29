//! The night look's specimen: its type and colours drawn by the launcher itself, over the sky's
//! stops, for checking the tokens in the real renderer (`HORIZON_LAUNCHER_PAGE=specimen`; not a
//! page players reach).
use crate::blocks::icon;
use crate::theme::{self, night, Theme};
use gpui::{div, prelude::*, px, rgb, rgba, Div, FontWeight, Hsla, Rgba};
use horizon_theme::{contrast, MICRO};
use std::sync::LazyLock;

/// Whether the launcher was opened on the specimen.
pub fn wanted() -> bool {
    static WANTED: LazyLock<bool> =
        LazyLock::new(|| std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("specimen"));
    *WANTED
}

/// Each sky stop's share of the window's height, top to bottom.
const BANDS: [f32; 6] = [0.2, 0.18, 0.17, 0.15, 0.15, 0.15];

fn alpha(c: u32, a: f32) -> Rgba {
    rgba(c << 8 | (a * 255.).round() as u32)
}

fn text(step: horizon_theme::Step, color: impl Into<Hsla>) -> Div {
    div()
        .font_family(if step.px >= night::DISPLAY.px {
            theme::DISPLAY_FONT
        } else {
            theme::FONT
        })
        .font_weight(FontWeight(step.weight as f32))
        .text_size(px(step.px))
        .line_height(px(step.line_px()))
        .text_color(color.into())
}

fn mono(color: Hsla) -> Div {
    div()
        .font_family(theme::MONO)
        .text_size(px(MICRO.px))
        .line_height(px(MICRO.line_px()))
        .text_color(color)
}

/// A plate over the sky: rounded, or with its corners cut in pixel steps (two steps of
/// [`night::PIXEL_CORNER`]), built of strips that don't overlap so the plate's opacity stays even.
fn plate(n: &night::Night, pixel: bool) -> Div {
    let fill = alpha(n.card, n.card_alpha);
    if !pixel {
        return div()
            .relative()
            .bg(fill)
            .rounded(px(night::RADIUS_PLATE))
            .border_1()
            .border_color(alpha(n.stroke, n.stroke_alpha));
    }
    let s = night::PIXEL_CORNER;
    let strip = |inset: f32| div().h(px(s)).mx(px(inset)).bg(fill);
    div()
        .relative()
        .flex()
        .flex_col()
        .child(strip(2. * s))
        .child(strip(s))
        .child(div().flex_1().bg(fill))
        .child(strip(s))
        .child(strip(2. * s))
}

pub fn render(t: &Theme) -> Div {
    let n = t.night;
    let ink: Hsla = rgb(n.ink).into();
    let ink_soft: Hsla = rgb(n.ink_soft).into();
    let muted: Hsla = rgb(n.muted).into();
    // The plate as it comes out over the brightest stop: the contrast figures are the worst case.
    let worst = night::over(n.card, n.card_alpha, n.sky[5]);
    let sky = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .flex_col()
        .children(n.sky.iter().zip(BANDS).map(|(&c, share)| {
            div()
                .w_full()
                .flex_grow()
                .flex_basis(gpui::relative(share))
                .bg(rgb(c))
        }));
    let weights = div().flex().gap(px(20.)).children(
        [
            ("Regular", 400.),
            ("Medium", 500.),
            ("SemiBold", 600.),
            ("Bold", 700.),
        ]
        .map(|(name, w)| {
            div()
                .font_family(theme::FONT)
                .font_weight(FontWeight(w))
                .text_size(px(18.))
                .text_color(ink)
                .child(format!("{name} {w}"))
        }),
    );
    let type_column = div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .w(px(600.))
        .child(mono(muted).child("horizon-theme · night · Bricolage Grotesque"))
        .child(text(night::DISPLAY, ink).child("Survival with friends"))
        .child(text(night::TITLE, ink).child("Java · Fabric 0.16.9 · 142 mods"))
        .child(
            text(night::UI, ink_soft)
                .child("Downloads 26.3 and Java 21, about 600 MB. Cancel any time: it picks up where it stopped."),
        )
        .child(text(night::CAPTION, muted).child("Last played 2 hours ago · 41 h in total · world “Spring Valley”"))
        .child(mono(muted).child("~/Library/Application Support/horizon/instances/survival-with-friends"))
        .child(weights)
        .child(text(night::UI, ink_soft).child("Zürich · Łódź · Ærøskøbing · Brașov · Ça va · 0123456789"))
        .child(
            div()
                .flex()
                .gap(px(10.))
                .items_baseline()
                .child(text(night::UI, ink_soft).child("Играть · Моды · Серверы"))
                .child(mono(muted).child("no Cyrillic upstream: system fallback")),
        );
    let swatch = |name: &'static str, c: u32| {
        let ratio = contrast(c, worst);
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(28.))
            .child(div().size(px(14.)).bg(rgb(c)))
            .child(text(night::UI, rgb(c)).w(px(110.)).child(name))
            .child(mono(muted).w(px(72.)).child(format!("#{c:06x}")))
            .child(
                mono(if ratio >= 4.5 {
                    ink_soft
                } else {
                    rgb(n.danger).into()
                })
                .child(format!("{ratio:.1}:1")),
            )
    };
    let button = |label: &'static str, fill: Option<Rgba>, color: Hsla| {
        div()
            .flex()
            .items_center()
            .h(px(theme::CONTROL))
            .px(px(14.))
            .rounded(px(night::RADIUS_CONTROL))
            .when_some(fill, |d, f| d.bg(f))
            .child(text(night::UI, color).child(label))
    };
    let chip = |label: &'static str, c: u32| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().size(px(6.)).bg(rgb(c)))
            .child(text(night::CAPTION, ink_soft).child(label))
    };
    let colours = plate(&n, false)
        .flex()
        .flex_col()
        .gap(px(4.))
        .w(px(440.))
        .p(px(24.))
        .child(text(night::TITLE, ink).child("Colours"))
        .child(
            mono(muted)
                .mb(px(8.))
                .child("contrast on a plate over the sunset's brightest stop"),
        )
        .children(
            [
                ("ink", n.ink),
                ("ink soft", n.ink_soft),
                ("muted", n.muted),
                ("accent hi", n.accent_hi),
                ("success", n.success),
                ("warn", n.warn),
                ("danger", n.danger),
            ]
            .map(|(name, c)| swatch(name, c)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .mt(px(18.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(night::PLAY))
                        .px(px(26.))
                        .rounded(px(night::RADIUS_CONTROL))
                        .bg(rgb(n.accent))
                        .child(icon("play", 20., rgb(n.on_accent).into()))
                        .child(text(night::TITLE, rgb(n.on_accent)).child("Play")),
                )
                .child(button(
                    "Spring Valley",
                    Some(alpha(n.card_hi, n.card_hi_alpha)),
                    ink,
                ))
                .child(
                    button("Folder", None, ink_soft)
                        .border_2()
                        .border_color(rgb(n.accent_hi)),
                ),
        )
        .child(
            div()
                .flex()
                .gap(px(16.))
                .mt(px(14.))
                .child(chip("Ready · 26.3", n.success))
                .child(chip("Update to 0.3.1", n.warn))
                .child(chip("Crashed: see log", n.danger)),
        );
    let corner = |pixel: bool, label: &'static str| {
        plate(&n, pixel).w(px(214.)).h(px(96.)).child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .justify_center()
                .px(px(18.))
                .child(text(night::UI, ink).child(label))
                .child(mono(muted).child(if pixel {
                    "4 px steps, crisp"
                } else {
                    "radius 12, 1 px stroke"
                })),
        )
    };
    let stops = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .children(n.sky.iter().enumerate().map(|(i, c)| {
            mono(if i < night::UNCARDED_STOPS {
                muted
            } else {
                ink
            })
            .child(format!("sky {i} #{c:06x}"))
        }));
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(sky)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .justify_between()
                .p(px(40.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_between()
                        .child(type_column)
                        .child(
                            div()
                                .flex()
                                .gap(px(16.))
                                .child(corner(false, "Rounded plate"))
                                .child(corner(true, "Pixel corner plate")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_between()
                        .items_end()
                        .child(colours)
                        .child(plate(&n, false).p(px(12.)).child(stops)),
                ),
        )
}
