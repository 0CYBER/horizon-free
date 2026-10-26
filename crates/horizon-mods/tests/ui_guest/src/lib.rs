//! A mod for the runtime's `ui` tests: `/ui_screen` opens a screen with one
//! of each widget, `/ui_settings` sets a settings page, `/ui_close` closes
//! the screen; the `ui-events` calls answer in chat so the host sees them.
//! `/render_hud`, `/render_clear` and `/render_shake` test `render`;
//! `/input_claim`, `/input_poll` and `/input_held` test `input`.
use horizon_mod_sdk::{
    Mod, Registration, chat,
    input::{self, Event},
    render::{self, FillShape, ItemShape, Point, QuadShape, Rect, Shape, SpriteShape, TextShape},
    ui::{self, ItemWidget, ListWidget, Screen, TextWidget, Widget},
};

/// One of each shape, the text too long by 44 characters.
fn shapes() -> Vec<Shape> {
    let (w, h) = render::gui_size();
    let p = |x, y| Point { x, y };
    vec![
        Shape::Fill(FillShape {
            rect: Rect {
                x: 4.0,
                y: 4.0,
                w: 60.0,
                h: 20.0,
            },
            color: 0x80FF0000,
        }),
        Shape::Quad(QuadShape {
            corners: (
                p(w / 2.0, 10.0),
                p(w / 2.0 + 20.0, 30.0),
                p(w / 2.0, 50.0),
                p(w / 2.0 - 20.0, 30.0),
            ),
            color: 0xC000FF00,
        }),
        Shape::Sprite(SpriteShape {
            sprite: "hud/heart/full".into(),
            rect: Rect {
                x: 70.0,
                y: 4.0,
                w: 18.0,
                h: 18.0,
            },
            color: 0xFFFFFFFF,
        }),
        Shape::Item(ItemShape {
            item: "minecraft:diamond".into(),
            at: p(92.0, 4.0),
            scale: 2.0,
        }),
        Shape::Text(TextShape {
            text: format!("gui {w}x{h} {}", "x".repeat(300)),
            at: p(w / 2.0, h / 2.0 - 30.0),
            color: 0xFFFF55,
            centered: true,
            shadow: true,
        }),
    ]
}

struct Guest;

fn widgets() -> Vec<Widget> {
    vec![
        Widget::Text(TextWidget {
            id: 1,
            text: "Pick a row".into(),
            color: 0,
        }),
        Widget::Item(ItemWidget {
            id: 2,
            item: "minecraft:diamond".into(),
            name: String::new(),
        }),
        Widget::List(ListWidget {
            id: 7,
            rows: vec!["first".into(), "second".into(), "third".into()],
            selected: Some(1),
        }),
    ]
}

impl Mod for Guest {
    fn init() -> Result<(Self, Registration), String> {
        let hooks = [
            "ui_screen",
            "ui_settings",
            "ui_close",
            "render_hud",
            "render_clear",
            "render_shake",
            "input_claim",
            "input_poll",
            "input_held",
        ]
        .into_iter()
        .fold(Registration::new(), Registration::command);
        Ok((Guest, hooks))
    }
    fn command(&mut self, name: &str, _args: &str) -> Result<Option<String>, String> {
        match name {
            "ui_screen" => ui::open_screen(&Screen {
                title: "Guest's screen".into(),
                widgets: widgets(),
            }),
            "ui_settings" => ui::set_settings(&widgets()),
            "ui_close" => ui::close_screen(),
            "render_hud" => render::set_hud(&shapes()),
            "render_clear" => render::set_hud(&[]),
            "input_claim" => {
                input::claim_items(&["minecraft:stick".into()]);
                input::register_key("wheel", "Open the wheel", "key.keyboard.r")?;
                // Not a key: refused.
                let bad = input::register_key("bad", "Bad", "R").unwrap_err();
                chat::show(&format!("claimed; {bad}"));
            }
            "input_poll" => {
                for event in input::poll() {
                    chat::show(&match event {
                        Event::UseStart(u) => format!("use-start {} {}", u.item.id, u.ticks),
                        Event::UseStop(u) => format!("use-stop {} {}", u.item.id, u.ticks),
                        Event::KeyDown(k) => format!("key-down {k}"),
                        Event::KeyUp(k) => format!("key-up {k}"),
                    });
                }
            }
            "input_held" => {
                let held = input::held().map_or("nothing".into(), |h| {
                    format!("{} x{} in {}", h.id, h.count, h.slot)
                });
                chat::show(&format!("holding {held}"));
            }
            // Clamped to (1, 10); the two after it shake nothing.
            "render_shake" => {
                render::shake(2.0, 99.0);
                render::shake(0.0, 1.0);
                render::shake(f32::NAN, 1.0);
            }
            _ => {}
        }
        Ok(None)
    }
    fn ui_select(&mut self, id: u32, row: u32) {
        chat::show(&format!("selected {id}:{row}"));
    }
    fn ui_closed(&mut self) {
        chat::show("closed");
    }
    fn ui_settings_opened(&mut self) {
        ui::set_settings(&[Widget::Text(TextWidget {
            id: 1,
            text: "fresh".into(),
            color: 0x33CC33,
        })]);
        chat::show("settings opened");
    }
}

horizon_mod_sdk::export_mod!(Guest);
