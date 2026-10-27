//! Sculk's banner: a server host that sponsors Horizon (Ludovic, 2026-10-26), large on the Servers page
//! and small at the foot of the sidebar on the other pages, where it shows without scrolling. The picture is in the binary, never fetched; a click opens Sculk's page in
//! the browser. A "Partner" label above it says it is an ad.
use crate::theme::Theme;
use gpui::{div, img, prelude::*, px, ClickEvent, Div, Image, ImageFormat, Stateful};
use std::sync::{Arc, LazyLock};

pub const URL: &str = "https://my.sculkhosting.com/ref/horizon";

/// How long the sidebar's banner stays on screen, counted over every start, before a "Hide" shows
/// beside its label: long enough to be seen, then the player may put it away for good.
pub const SEEN_BEFORE_HIDE: u64 = 180;

/// 1200×441: twice the widest it is shown at, for Retina screens. Its transparent edges let the
/// characters stand out of the card.
const WIDTH: f32 = 1200.;
const HEIGHT: f32 = 441.;
static BANNER: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Webp,
        include_bytes!("../assets/sculk-banner.webp").to_vec(),
    ))
});

/// The banner `width` logical pixels wide, under its label. Hovering lifts it a little and turns the
/// label to ink. The narrowest window (960) leaves the page 648 pixels, so the Servers one always fits.
pub fn banner(id: &'static str, width: f32, t: &Theme) -> Stateful<Div> {
    let t = *t;
    let el = div()
        .id(id)
        .group(id)
        .flex()
        .flex_col()
        .gap(px(8.))
        .w(px(width))
        .cursor_pointer()
        .child(
            crate::text(crate::theme::MICRO)
                .text_color(t.muted)
                .group_hover(id, move |s| s.text_color(t.ink))
                .child("PARTNER"),
        )
        .child(
            div().relative().group_hover(id, |s| s.top(px(-2.))).child(
                img(BANNER.clone())
                    .w(px(width))
                    .h(px(width * HEIGHT / WIDTH)),
            ),
        )
        .on_click(|_: &ClickEvent, _, cx| cx.open_url(URL));
    crate::keyboard(el, &t)
}

#[cfg(test)]
mod tests {
    use super::{HEIGHT, WIDTH};

    #[test]
    fn the_bundled_banner_decodes() {
        let bytes = include_bytes!("../assets/sculk-banner.webp");
        let banner = image::load_from_memory_with_format(bytes, image::ImageFormat::WebP)
            .expect("the banner is a WebP");
        assert_eq!(
            (banner.width(), banner.height()),
            (WIDTH as u32, HEIGHT as u32)
        );
    }
}
