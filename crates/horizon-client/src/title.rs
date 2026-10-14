//! Title, world selection and world creation screens (TitleScreen,
//! SelectWorldScreen, CreateWorldScreen layouts), drawn as HUD quads.
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    save::Level,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct WorldEntry {
    pub folder: String,
    pub name: String,
    pub creative: bool,
    pub modified: std::time::SystemTime,
}
#[derive(Clone, Debug)]
pub enum Mode {
    Main,
    Select {
        worlds: Vec<WorldEntry>,
        selected: Option<usize>,
    },
    Create {
        name: String,
        seed: String,
        creative: bool,
        focus: u8,
    },
    Multiplayer(crate::multiplayer::Screen),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Options,
    Play {
        folder: String,
    },
    Create {
        folder: String,
        name: String,
        seed: Option<i64>,
        creative: bool,
    },
    Connect {
        address: String,
    },
}
pub struct Title {
    pub mode: Mode,
    pub saves: PathBuf,
    pub started: std::time::Instant,
    /// The mode last drawn and when it appeared (Horizon's screen fades).
    shown: std::cell::Cell<(std::mem::Discriminant<Mode>, std::time::Instant)>,
    /// The launcher-style opening (Horizon style, first title of the session).
    pub opening: std::cell::Cell<Option<crate::front::Opening>>,
    /// The saves listed for Home, re-read every couple of seconds.
    recent: std::cell::RefCell<Option<(std::time::Instant, Vec<WorldEntry>)>>,
}
struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: String,
    enabled: bool,
    id: u8,
}
/// Java String.hashCode for text seeds (WorldOptions.parseSeed).
pub fn parse_seed(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    Some(t.parse::<i64>().unwrap_or_else(|_| {
        i64::from(
            t.encode_utf16()
                .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c))),
        )
    }))
}
pub fn list_worlds(saves: &Path) -> Vec<WorldEntry> {
    let mut worlds: Vec<WorldEntry> = std::fs::read_dir(saves)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let level = Level::read(&path)?;
            let modified = std::fs::metadata(path.join("level.dat"))
                .and_then(|m| m.modified())
                .ok()?;
            Some(WorldEntry {
                folder: e.file_name().to_string_lossy().into_owned(),
                name: level.name,
                creative: level.creative,
                modified,
            })
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.modified));
    worlds
}
impl Title {
    pub fn new(saves: PathBuf) -> Self {
        let mode = Mode::Main;
        let now = std::time::Instant::now();
        Self {
            shown: std::cell::Cell::new((std::mem::discriminant(&mode), now)),
            mode,
            saves,
            started: now,
            opening: std::cell::Cell::new(None),
            recent: std::cell::RefCell::new(None),
        }
    }
    /// The worlds, newest first, cached for two seconds (drawn every frame).
    pub fn recent_worlds(&self) -> Vec<WorldEntry> {
        let mut recent = self.recent.borrow_mut();
        match &*recent {
            Some((at, worlds)) if at.elapsed().as_secs() < 2 => worlds.clone(),
            _ => {
                let worlds = list_worlds(&self.saves);
                *recent = Some((std::time::Instant::now(), worlds.clone()));
                worlds
            }
        }
    }
    /// When the current mode first appeared.
    pub fn entered(&self) -> std::time::Instant {
        let now = std::mem::discriminant(&self.mode);
        let (mode, at) = self.shown.get();
        if mode == now {
            return at;
        }
        let at = std::time::Instant::now();
        self.shown.set((now, at));
        at
    }
    /// Create New World: the folder name and options (FileUtil.findAvailableName).
    pub fn create_action(&self) -> Action {
        let Mode::Create {
            name,
            seed,
            creative,
            ..
        } = &self.mode
        else {
            return Action::None;
        };
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = if base.trim().is_empty() {
            "World".to_owned()
        } else {
            base.trim().to_owned()
        };
        // FileUtil.findAvailableName: append (n) until unused.
        let mut folder = base.clone();
        let mut n = 1;
        while self.saves.join(&folder).exists() {
            folder = format!("{base} ({n})");
            n += 1;
        }
        Action::Create {
            folder,
            name: name.clone(),
            seed: parse_seed(seed),
            creative: *creative,
        }
    }
    fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        let b = |x: f32, y: f32, w: f32, label: &str, enabled: bool, id: u8| Button {
            x,
            y,
            w,
            label: label.into(),
            enabled,
            id,
        };
        match &self.mode {
            Mode::Main => {
                let y = h / 4.0 + 48.0;
                vec![
                    b(w / 2.0 - 100.0, y, 200.0, "Singleplayer", true, 1),
                    b(w / 2.0 - 100.0, y + 24.0, 200.0, "Multiplayer", true, 4),
                    b(
                        w / 2.0 - 100.0,
                        y + 48.0,
                        200.0,
                        "Minecraft Realms",
                        false,
                        0,
                    ),
                    b(w / 2.0 - 100.0, y + 84.0, 98.0, "Options...", true, 3),
                    b(w / 2.0 + 2.0, y + 84.0, 98.0, "Quit Game", true, 2),
                ]
            }
            Mode::Select { selected, .. } => vec![
                b(
                    w / 2.0 - 154.0,
                    h - 52.0,
                    150.0,
                    "Play Selected World",
                    selected.is_some(),
                    10,
                ),
                b(w / 2.0 + 4.0, h - 52.0, 150.0, "Create New World", true, 11),
                b(w / 2.0 + 4.0, h - 28.0, 150.0, "Back", true, 12),
            ],
            Mode::Create { creative, .. } => vec![
                b(
                    w / 2.0 - 75.0,
                    120.0,
                    150.0,
                    if *creative {
                        "Game Mode: Creative"
                    } else {
                        "Game Mode: Survival"
                    },
                    true,
                    20,
                ),
                b(
                    w / 2.0 - 155.0,
                    h - 28.0,
                    150.0,
                    "Create New World",
                    true,
                    21,
                ),
                b(w / 2.0 + 5.0, h - 28.0, 150.0, "Cancel", true, 22),
            ],
            Mode::Multiplayer(screen) => screen
                .buttons(w, h)
                .into_iter()
                .map(|(x, y, bw, label, enabled, id)| b(x, y, bw, &label, enabled, id))
                .collect(),
        }
    }
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        match &self.mode {
            Mode::Main => {
                // Logo: minecraft.png is a 1024x256 sheet drawn at 256x64.
                batch.sprite_scaled(
                    atlas,
                    "gui/title/minecraft",
                    w / 2.0 - 128.0,
                    30.0,
                    256.0,
                    64.0,
                    u32::MAX,
                );
                batch.sprite_scaled(
                    atlas,
                    "gui/title/edition",
                    w / 2.0 - 64.0,
                    30.0 + 64.0 - 7.0,
                    128.0,
                    14.0,
                    u32::MAX,
                );
                let version = "Horizon 26.3 (Rust)";
                batch.text(atlas, version, 2.0, h - 10.0, u32::MAX, true);
                let notice = "Not affiliated with Mojang or Microsoft";
                let tw = HudBatch::text_width(atlas, notice);
                batch.text(atlas, notice, w - tw - 2.0, h - 10.0, u32::MAX, true);
            }
            Mode::Select { worlds, selected } => {
                batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 120));
                let title = "Select World";
                let tw = HudBatch::text_width(atlas, title);
                batch.text(atlas, title, (w - tw) / 2.0, 8.0, u32::MAX, true);
                batch.fill(atlas, 0.0, 32.0, w, h - 32.0 - 64.0, rgba(0, 0, 0, 100));
                for (i, world) in worlds.iter().enumerate() {
                    let y = 36.0 + i as f32 * 36.0;
                    if y + 36.0 > h - 64.0 {
                        break;
                    }
                    let x = w / 2.0 - 110.0;
                    if *selected == Some(i) {
                        batch.fill(
                            atlas,
                            x - 2.0,
                            y - 2.0,
                            224.0,
                            36.0,
                            rgba(128, 128, 128, 255),
                        );
                        batch.fill(atlas, x - 1.0, y - 1.0, 222.0, 34.0, rgba(0, 0, 0, 255));
                    }
                    batch.text(atlas, &world.name, x + 34.0, y + 1.0, u32::MAX, false);
                    let date = world
                        .modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs());
                    let line = format!("{} ({})", world.folder, date_string(date));
                    batch.text(
                        atlas,
                        &line,
                        x + 34.0,
                        y + 12.0,
                        rgba(128, 128, 128, 255),
                        false,
                    );
                    let mode = if world.creative {
                        "Creative Mode"
                    } else {
                        "Survival Mode"
                    };
                    batch.text(
                        atlas,
                        mode,
                        x + 34.0,
                        y + 22.0,
                        rgba(128, 128, 128, 255),
                        false,
                    );
                    batch.sprite_scaled(atlas, "misc/unknown_server", x, y, 32.0, 32.0, u32::MAX);
                }
                if worlds.is_empty() {
                    let empty = "No worlds yet: create one!";
                    let tw = HudBatch::text_width(atlas, empty);
                    batch.text(
                        atlas,
                        empty,
                        (w - tw) / 2.0,
                        h / 2.0 - 20.0,
                        rgba(160, 160, 160, 255),
                        true,
                    );
                }
            }
            Mode::Create {
                name, seed, focus, ..
            } => {
                batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 120));
                let title = "Create New World";
                let tw = HudBatch::text_width(atlas, title);
                batch.text(atlas, title, (w - tw) / 2.0, 8.0, u32::MAX, true);
                let blink = (self.started.elapsed().as_millis() / 300).is_multiple_of(2);
                for (i, (label, value)) in [
                    ("World Name", name),
                    (
                        "Seed for the World Generator (leave blank for random)",
                        seed,
                    ),
                ]
                .iter()
                .enumerate()
                {
                    let y = 40.0 + i as f32 * 40.0;
                    batch.text(
                        atlas,
                        label,
                        w / 2.0 - 100.0,
                        y,
                        rgba(160, 160, 160, 255),
                        true,
                    );
                    batch.fill(
                        atlas,
                        w / 2.0 - 100.0,
                        y + 11.0,
                        200.0,
                        20.0,
                        if *focus == i as u8 {
                            u32::MAX
                        } else {
                            rgba(160, 160, 160, 255)
                        },
                    );
                    batch.fill(
                        atlas,
                        w / 2.0 - 99.0,
                        y + 12.0,
                        198.0,
                        18.0,
                        rgba(0, 0, 0, 255),
                    );
                    let shown = if *focus == i as u8 && blink {
                        format!("{value}_")
                    } else {
                        value.to_string()
                    };
                    batch.text(
                        atlas,
                        &shown,
                        w / 2.0 - 96.0,
                        y + 17.0,
                        rgba(224, 224, 224, 255),
                        true,
                    );
                }
            }
            Mode::Multiplayer(screen) => screen.draw(
                atlas,
                batch,
                (self.started.elapsed().as_millis() / 300).is_multiple_of(2),
            ),
        }
        for b in self.buttons(w, h) {
            crate::widgets::record(&b.label, [b.x, b.y, b.w, 20.0], b.enabled);
            let hover = b.enabled
                && cursor.0 >= b.x
                && cursor.0 < b.x + b.w
                && cursor.1 >= b.y
                && cursor.1 < b.y + 20.0;
            let sprite = if !b.enabled {
                "gui/sprites/widget/button_disabled"
            } else if hover {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            };
            // Nine-slice approximation: left and right halves of the 200-wide sprite.
            if let Some(r) = atlas.sprites.get(sprite).copied() {
                let half = b.w / 2.0;
                let mut left = r;
                left.u1 = r.u0 + (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(b.x, b.y, half, 20.0, left, u32::MAX);
                let mut right = r;
                right.u0 = r.u1 - (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(b.x + half, b.y, half, 20.0, right, u32::MAX);
            }
            let color = if b.enabled {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            };
            let tw = HudBatch::text_width(atlas, &b.label);
            batch.text(
                atlas,
                &b.label,
                b.x + (b.w - tw) / 2.0,
                b.y + 6.0,
                color,
                true,
            );
        }
    }
    pub fn click(&mut self, x: f32, y: f32, size: (f32, f32)) -> Action {
        let (w, h) = size;
        let hit = self
            .buttons(w, h)
            .into_iter()
            .find(|b| b.enabled && x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + 20.0)
            .map(|b| b.id);
        match (&mut self.mode, hit) {
            (Mode::Main, Some(1)) => {
                self.mode = Mode::Select {
                    worlds: list_worlds(&self.saves),
                    selected: None,
                };
                Action::None
            }
            (Mode::Main, Some(2)) => Action::Quit,
            (Mode::Main, Some(4)) => {
                self.mode = Mode::Multiplayer(crate::multiplayer::Screen::list(&self.saves));
                Action::None
            }
            (Mode::Multiplayer(screen), hit) => {
                match screen.click(hit, x, y, (w, h), &self.saves) {
                    crate::multiplayer::Outcome::Back => {
                        self.mode = Mode::Main;
                        Action::None
                    }
                    crate::multiplayer::Outcome::Connect(address) => Action::Connect { address },
                    crate::multiplayer::Outcome::None => Action::None,
                }
            }
            (Mode::Main, Some(3)) => Action::Options,
            (Mode::Select { worlds, selected }, Some(10)) => {
                match selected.and_then(|i| worlds.get(i)) {
                    Some(world) => Action::Play {
                        folder: world.folder.clone(),
                    },
                    None => Action::None,
                }
            }
            (Mode::Select { .. }, Some(11)) => {
                self.mode = Mode::Create {
                    name: "New World".into(),
                    seed: String::new(),
                    creative: false,
                    focus: 0,
                };
                Action::None
            }
            (Mode::Select { .. }, Some(12)) => {
                self.mode = Mode::Main;
                Action::None
            }
            (Mode::Select { worlds, selected }, None) => {
                // Clicking a list entry selects it; clicking it again plays it.
                let index = ((y - 36.0) / 36.0).floor();
                if y >= 36.0
                    && y < h - 64.0
                    && (x - w / 2.0).abs() < 112.0
                    && (index as usize) < worlds.len()
                {
                    let index = index as usize;
                    if *selected == Some(index) {
                        return Action::Play {
                            folder: worlds[index].folder.clone(),
                        };
                    }
                    *selected = Some(index);
                }
                Action::None
            }
            (Mode::Create { creative, .. }, Some(20)) => {
                *creative = !*creative;
                Action::None
            }
            (Mode::Create { .. }, Some(21)) => self.create_action(),
            (Mode::Create { .. }, Some(22)) => {
                self.mode = Mode::Select {
                    worlds: list_worlds(&self.saves),
                    selected: None,
                };
                Action::None
            }
            (Mode::Create { focus, .. }, None) => {
                for i in 0..2u8 {
                    let fy = 40.0 + f32::from(i) * 40.0 + 11.0;
                    if y >= fy && y < fy + 20.0 && (x - w / 2.0).abs() < 100.0 {
                        *focus = i;
                    }
                }
                Action::None
            }
            _ => Action::None,
        }
    }
    /// The focused text field, its id and maximum length (copy and paste).
    pub fn focused_field(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        match &mut self.mode {
            Mode::Multiplayer(screen) => screen.focused_text(),
            Mode::Create {
                name, seed, focus, ..
            } => match focus {
                0 => Some((("world.name", 0), name, 32)),
                1 => Some((("world.seed", 0), seed, 32)),
                _ => None,
            },
            _ => None,
        }
    }
    pub fn type_text(&mut self, text: &str) {
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.type_text(text);
        }
        if let Mode::Create {
            name, seed, focus, ..
        } = &mut self.mode
            && *focus < 2
        {
            let field = if *focus == 0 { name } else { seed };
            for c in text.chars().filter(|c| !c.is_control()) {
                if field.chars().count() < 32 {
                    field.push(c);
                }
            }
        }
    }
    pub fn backspace(&mut self) {
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.backspace();
        }
        if let Mode::Create {
            name, seed, focus, ..
        } = &mut self.mode
            && *focus < 2
        {
            let field = if *focus == 0 { name } else { seed };
            field.pop();
        }
    }
    pub fn tab(&mut self) {
        if let Mode::Multiplayer(screen) = &mut self.mode {
            screen.tab();
        }
        if let Mode::Create { focus, .. } = &mut self.mode {
            *focus = (*focus + 1) % 2;
        }
    }
    /// The server auto-reconnect retries now, once its countdown is over.
    pub fn due_retry(&mut self) -> Option<String> {
        match &mut self.mode {
            Mode::Multiplayer(screen) => screen.due_retry(),
            _ => None,
        }
    }
    pub fn escape(&mut self) {
        if let Mode::Multiplayer(screen) = &mut self.mode
            && !screen.escape(&self.saves)
        {
            return;
        }
        self.mode = match &self.mode {
            Mode::Create { .. } => Mode::Select {
                worlds: list_worlds(&self.saves),
                selected: None,
            },
            _ => Mode::Main,
        };
    }
}
/// `yyyy/MM/dd HH:mm` (UTC) for a modification time in Unix seconds.
pub fn date_label(secs: u64) -> String {
    date_string(secs)
}
fn date_string(secs: u64) -> String {
    // Civil date from Unix time (days-from-civil inverse), UTC.
    let days = (secs / 86400) as i64;
    let (h, m) = ((secs % 86400) / 3600, (secs % 3600) / 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y}-{mo:02}-{d:02} {h:02}:{m:02}")
}
