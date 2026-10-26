//! Horizon's official keystrokes overlay, formerly the built-in extra:
//! forward, left, back and right as the player bound them, the two mouse
//! buttons (with clicks per second) and the space bar, lit while held,
//! drawn as the extra drew them. It redraws in the frame hook only when
//! something changed. Settings (`corner`, `cps`) live in the mod's storage.
use horizon_mod_sdk::{
    Mod, Registration, input,
    render::{self, Corner, FillShape, Rect, Shape, TextShape},
    storage,
    ui::{self, ButtonWidget, Widget},
};

const BACKGROUND: u32 = 0x7800_0000;
/// A held key: light fill, dark text without a shadow.
const HELD: u32 = 0xB0FF_FFFF;
const INK_HELD: u32 = 0x14_1414;
const INK: u32 = 0xFF_FFFF;
const KEY: f32 = 22.0;
const GAP: f32 = 2.0;
const CORNERS: [(Corner, &str, &str); 4] = [
    (Corner::TopLeft, "top_left", "Top left"),
    (Corner::TopRight, "top_right", "Top right"),
    (Corner::BottomLeft, "bottom_left", "Bottom left"),
    (Corner::BottomRight, "bottom_right", "Bottom right"),
];
const CORNER_BUTTON: u32 = 1;
const CPS_BUTTON: u32 = 2;
const MOVES: [&str; 4] = ["key.forward", "key.left", "key.back", "key.right"];

/// What the overlay shows; a change redraws it.
#[derive(Clone, PartialEq, Default)]
struct View {
    moves: [(bool, String); 4],
    jump: bool,
    attack: bool,
    use_: bool,
    cps: Option<(u32, u32)>,
}

struct Keystrokes {
    corner: usize,
    cps: bool,
    shown: Option<View>,
}

fn down(name: &str) -> (bool, String) {
    input::mapping(name).map_or((false, String::new()), |m| (m.down, m.label))
}

/// The extra's key box: a fill and its lines centred, 9 pixels apart.
fn key_box(out: &mut Vec<Shape>, [x, y, w, h]: [f32; 4], held: bool, lines: &[&str]) {
    let (fill, ink) = if held {
        (HELD, INK_HELD)
    } else {
        (BACKGROUND, INK)
    };
    out.push(Shape::Fill(FillShape {
        rect: Rect { x, y, w, h },
        color: fill,
    }));
    let total = lines.len() as f32 * 9.0 - 1.0;
    for (i, line) in lines.iter().enumerate() {
        let tw = render::text_width(line);
        out.push(Shape::Text(TextShape {
            text: (*line).into(),
            at: render::Point {
                x: (x + (w - tw) / 2.0).round(),
                y: (y + (h - total) / 2.0).round() + i as f32 * 9.0,
            },
            color: ink,
            centered: false,
            shadow: !held,
        }));
    }
}

impl Keystrokes {
    fn view(&self) -> View {
        let (l, r) = input::clicks_per_second();
        View {
            moves: MOVES.map(down),
            jump: down("key.jump").0,
            attack: down("key.attack").0,
            use_: down("key.use").0,
            cps: self.cps.then_some((l, r)),
        }
    }
    fn draw(&mut self) {
        let v = self.view();
        if self.shown.as_ref() == Some(&v) {
            return;
        }
        let (w, h) = (3.0 * KEY + 2.0 * GAP, 3.0 * KEY + 3.0 * GAP + 10.0);
        let step = KEY + GAP;
        let mut out = Vec::with_capacity(24);
        let [forward, left, back, right] = &v.moves;
        key_box(&mut out, [step, 0.0, KEY, KEY], forward.0, &[&forward.1]);
        for (i, key) in [left, back, right].into_iter().enumerate() {
            key_box(
                &mut out,
                [i as f32 * step, step, KEY, KEY],
                key.0,
                &[&key.1],
            );
        }
        let mw = (w - GAP) / 2.0;
        let my = 2.0 * step;
        for (i, (held, label, cps)) in [
            (v.attack, "LMB", v.cps.map(|c| c.0)),
            (v.use_, "RMB", v.cps.map(|c| c.1)),
        ]
        .into_iter()
        .enumerate()
        {
            let rate = cps.map(|n| format!("{n} CPS"));
            let lines: Vec<&str> = std::iter::once(label).chain(rate.as_deref()).collect();
            key_box(&mut out, [i as f32 * (mw + GAP), my, mw, KEY], held, &lines);
        }
        // The space bar: a line in a short key.
        let sy = my + step;
        key_box(&mut out, [0.0, sy, w, 10.0], v.jump, &[]);
        out.push(Shape::Fill(FillShape {
            rect: Rect {
                x: w / 2.0 - 12.0,
                y: sy + 4.0,
                w: 24.0,
                h: 2.0,
            },
            color: 0xFF00_0000 | if v.jump { INK_HELD } else { INK },
        }));
        render::set_corner(CORNERS[self.corner].0, w, h, &out);
        self.shown = Some(v);
    }
    /// The settings page: the screen's title is the mod's name already.
    fn page(&self) {
        let on = |b: bool| if b { "ON" } else { "OFF" };
        ui::set_settings(&[
            Widget::Button(ButtonWidget {
                id: CORNER_BUTTON,
                label: format!("Corner: {}", CORNERS[self.corner].2),
            }),
            Widget::Button(ButtonWidget {
                id: CPS_BUTTON,
                label: format!("Clicks per second: {}", on(self.cps)),
            }),
        ]);
    }
    fn save(&self) {
        let _ = storage::set("corner", CORNERS[self.corner].1.as_bytes());
        let _ = storage::set("cps", if self.cps { b"true" } else { b"false" });
    }
}

fn stored(key: &str) -> Option<String> {
    storage::get(key).and_then(|v| String::from_utf8(v).ok())
}

impl Mod for Keystrokes {
    fn init() -> Result<(Self, Registration), String> {
        // The extra's defaults: top left, with clicks per second.
        let corner = stored("corner")
            .and_then(|c| CORNERS.iter().position(|k| k.1 == c.trim()))
            .unwrap_or(0);
        let cps = stored("cps").is_none_or(|v| v.trim() != "false");
        let k = Keystrokes {
            corner,
            cps,
            shown: None,
        };
        k.page();
        Ok((k, Registration::new().on_frame()))
    }
    fn frame(&mut self, _seconds: f32) {
        self.draw();
    }
    fn ui_select(&mut self, id: u32, _row: u32) {
        match id {
            CORNER_BUTTON => self.corner = (self.corner + 1) % CORNERS.len(),
            CPS_BUTTON => self.cps = !self.cps,
            _ => return,
        }
        self.save();
        self.page();
        self.shown = None;
        self.draw();
    }
    fn ui_settings_opened(&mut self) {
        self.page();
    }
}

horizon_mod_sdk::export_mod!(Keystrokes);
