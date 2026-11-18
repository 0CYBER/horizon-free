//! Mod API GUI commands to and from the engine's: every variant, field for field.
use super::{TryIntoApi, TryIntoEngine};

macro_rules! gui_draw_value {
    ($draw:expr, $from:ident, $to:ident, $convert:ident($($args:expr),*)) => {
        match $draw {
            $from::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                region,
                color,
                blend,
            } => $to::Sprite {
                sprite: sprite.clone(),
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                region: *region,
                color: *color,
                blend: *blend,
            },
            $from::AtlasSprite {
                sprite,
                x,
                y,
                w,
                h,
                color,
            } => $to::AtlasSprite {
                sprite: sprite.clone(),
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                color: *color,
            },
            $from::Blit {
                texture,
                blend,
                x,
                y,
                w,
                h,
                uv,
                src,
                tex,
                color,
            } => $to::Blit {
                texture: texture.clone(),
                blend: *blend,
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                uv: *uv,
                src: *src,
                tex: *tex,
                color: *color,
            },
            $from::Fill {
                x0,
                y0,
                x1,
                y1,
                color,
            } => $to::Fill {
                x0: *x0,
                y0: *y0,
                x1: *x1,
                y1: *y1,
                color: *color,
            },
            $from::FillGradient {
                x0,
                y0,
                x1,
                y1,
                top,
                bottom,
            } => $to::FillGradient {
                x0: *x0,
                y0: *y0,
                x1: *x1,
                y1: *y1,
                top: *top,
                bottom: *bottom,
            },
            $from::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => $to::Text {
                text: text.$convert($($args),*)?,
                x: *x,
                y: *y,
                color: *color,
                shadow: *shadow,
            },
            $from::Item {
                slot,
                x,
                y,
                seed,
                scale,
                pivot,
            } => $to::Item {
                slot: *slot,
                x: *x,
                y: *y,
                seed: *seed,
                scale: *scale,
                pivot: *pivot,
            },
            $from::Push(ops) => $to::Push(ops.clone()),
            $from::Pop => $to::Pop,
        }
    };
}

impl TryIntoApi for horizon_client::hud_layout::GuiDraw {
    type Api = mod_api::client::hud_layout::GuiDraw;
    type Error = mod_api::text::TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        use horizon_client::hud_layout::GuiDraw as Engine;
        use mod_api::client::hud_layout::GuiDraw as Api;
        Ok(gui_draw_value!(self, Engine, Api, try_into_api()))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for mod_api::client::hud_layout::GuiDraw {
    type Engine = horizon_client::hud_layout::GuiDraw;
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, _: &C) -> Result<Self::Engine, Self::Error> {
        use horizon_client::hud_layout::GuiDraw as Engine;
        use mod_api::client::hud_layout::GuiDraw as Api;
        Ok(gui_draw_value!(self, Api, Engine, try_into_engine(&())))
    }
}
