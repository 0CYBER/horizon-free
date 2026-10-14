//! The front-end screens in Horizon's menu style (`ui_theme`), laid out like
//! the launcher: a 200-wide sidebar with the sliding selection (Home,
//! Singleplayer, Multiplayer, Options), content pages with a page header,
//! settings rows and 1px dividers, and the launcher's opening (the wordmark,
//! then the interface). They drive the same state as the classic layouts in
//! `title` and `multiplayer`; only drawing and hit testing differ. The pause
//! menu's Horizon layout is `pause`, below.
use crate::{
    hud::{HudAtlas, HudBatch},
    multiplayer::{Outcome, Ping, Screen},
    title::{Action, Mode, Title, list_worlds},
    ui_theme::{
        self as t, BODY, CONTROL, CONTROL_LARGE, Face, Kind, SIDEBAR, SMALL, Ui, color, id,
    },
};
use std::time::Instant;

const NAV: u64 = id("title.nav", 0);
const QUIT: u64 = id("title.quit", 0);
const PLAY: u64 = id("home.play", 0);
const MULTIPLAYER: u64 = id("home.multiplayer", 0);
const RESUME: u64 = id("home.resume", 0);
const BACK: u64 = id("front.back", 0);
const PLAY_WORLD: u64 = id("worlds.play", 0);
const CREATE: u64 = id("worlds.create", 0);
const WORLD: u64 = id("worlds.row", 0);
const NAME: u64 = id("create.name", 0);
const SEED: u64 = id("create.seed", 0);
const MODE: u64 = id("create.mode", 0);
const MAKE: u64 = id("create.make", 0);
const SERVER: u64 = id("mp.row", 0);
const JOIN: u64 = id("mp.join", 0);
const DIRECT: u64 = id("mp.direct", 0);
const ADD: u64 = id("mp.add", 0);
const EDIT: u64 = id("mp.edit", 0);
const DELETE: u64 = id("mp.delete", 0);
const REFRESH: u64 = id("mp.refresh", 0);
const FIELD_A: u64 = id("mp.field", 0);
const FIELD_B: u64 = id("mp.field", 1);
const DONE: u64 = id("mp.done", 0);
const CANCEL_RETRY: u64 = id("mp.cancel_retry", 0);

/// The launcher's opening (`launcher/src/main.rs`): the wordmark holds over
/// the backdrop, then the interface comes in. A click or a key skips ahead.
#[derive(Clone, Copy, Debug)]
pub struct Opening {
    started: Instant,
    reveal: Option<Instant>,
}
impl Opening {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            reveal: None,
        }
    }
    /// The reveal's progress, 0 to 1; `ready` says the backdrop is showing.
    fn phase(&mut self, ready: bool) -> f32 {
        let held = self.started.elapsed();
        if self.reveal.is_none()
            && ((held >= horizon_theme::OPENING_HOLD && ready)
                || held >= horizon_theme::OPENING_MAX)
        {
            self.reveal = Some(Instant::now());
        }
        self.reveal.map_or(0.0, |at| {
            at.elapsed().as_secs_f32() / horizon_theme::OPENING_REVEAL.as_secs_f32()
        })
    }
    pub fn skip(&mut self) {
        self.reveal.get_or_insert_with(Instant::now);
    }
}
impl Default for Opening {
    fn default() -> Self {
        Self::new()
    }
}

/// Which sidebar item a mode belongs to.
fn page(mode: &Mode) -> usize {
    match mode {
        Mode::Main => 0,
        Mode::Select { .. } | Mode::Create { .. } => 1,
        Mode::Multiplayer(_) => 2,
    }
}
const PAGES: [&str; 4] = ["Home", "Singleplayer", "Multiplayer", "Options"];

impl Title {
    /// Horizon's layout for every title mode. `backdrop` is how much of the
    /// live backdrop is visible (0 shows the plain cold background).
    pub fn draw_horizon(&self, ui: &mut Ui, backdrop: f32) {
        let reveal = self.opening_phase(backdrop >= 1.0);
        if backdrop < 1.0 {
            ui.cold_background(1.0 - backdrop);
        }
        let shade = t::ease_out_cubic(reveal);
        let veil = if matches!(self.mode, Mode::Main) {
            0.42
        } else {
            0.66
        };
        ui.atmosphere(veil * shade, shade);
        let side = t::ease_out_cubic((reveal - 0.2) / 0.7);
        let main = t::ease_out_cubic((reveal - 0.35) / 0.65);

        // The sidebar slides in from the left.
        ui.opacity = side;
        ui.offset = (-16.0 * (1.0 - side), 0.0);
        if side > 0.0 {
            self.sidebar(ui);
        }
        // The content rises; each new page fades in and rises 8 over MOVE.
        let entered = self.entered();
        let p = t::ease_out_quint(entered.elapsed().as_secs_f32() / t::move_());
        ui.opacity = main * p;
        ui.offset = (0.0, 16.0 * (1.0 - main) + 8.0 * (1.0 - p));
        if main > 0.0 {
            let x = SIDEBAR + 48.0;
            let w = (ui.w - x - 48.0).min(760.0);
            match &self.mode {
                Mode::Main => self.home(ui, x, w),
                Mode::Select { worlds, selected } => worlds_page(ui, x, w, worlds, *selected),
                Mode::Create {
                    name,
                    seed,
                    creative,
                    focus,
                } => {
                    let caret = (self.started.elapsed().as_millis() / 500).is_multiple_of(2);
                    create_page(ui, x, w, name, seed, *creative, *focus, caret);
                }
                Mode::Multiplayer(screen) => {
                    let caret = (self.started.elapsed().as_millis() / 500).is_multiple_of(2);
                    multiplayer_page(ui, x, w, screen, caret);
                }
            }
        }
        ui.opacity = 1.0;
        ui.offset = (0.0, 0.0);
        if reveal < 1.0 {
            // The wordmark rises and fades as the interface comes in.
            let word = 1.0 - t::ease_out_cubic(reveal / 0.6);
            ui.opacity = word;
            let tw = ui.measure(Face::Display, t::WORDMARK, "Horizon");
            let line = (t::WORDMARK * 1.2).round();
            ui.text_line(
                Face::Display,
                t::WORDMARK,
                line,
                ((ui.w - tw) / 2.0).round(),
                (ui.h - line) / 2.0 - 28.0 * (1.0 - word),
                color::WHITE,
                "Horizon",
            );
            ui.opacity = 1.0;
        }
    }
    fn opening_phase(&self, ready: bool) -> f32 {
        let mut opening = self.opening.get();
        let phase = opening.as_mut().map_or(1.0, |o| o.phase(ready));
        self.opening.set(opening.filter(|_| phase < 1.0));
        phase.min(1.0)
    }
    /// A click or key during the opening jumps to the reveal; true if it did.
    pub fn skip_opening(&self) -> bool {
        let mut opening = self.opening.get();
        let skipped = opening.as_mut().is_some_and(|o| {
            let holding = o.reveal.is_none();
            o.skip();
            holding
        });
        self.opening.set(opening);
        skipped
    }
    fn sidebar(&self, ui: &mut Ui) {
        let h = ui.h;
        ui.fill(0.0, 0.0, SIDEBAR, h, color::with_alpha(color::SIDEBAR, 0.8));
        ui.vline(SIDEBAR - 1.0, 0.0, h, color::BORDER);
        ui.text_line(
            Face::Display,
            22.0,
            30.0,
            22.0,
            16.0,
            color::TEXT,
            "Horizon",
        );
        ui.sidebar_list(NAV, 12.0, 66.0, SIDEBAR - 24.0, &PAGES, page(&self.mode));
        let qy = h - 12.0 - 18.0 - 12.0 - CONTROL;
        let hover = ui.interact(QUIT, 12.0, qy, SIDEBAR - 24.0, CONTROL, true);
        ui.rounded(
            12.0,
            qy,
            SIDEBAR - 24.0,
            CONTROL,
            t::RADIUS,
            color::with_alpha(color::HOVER, 0.7 * hover),
        );
        ui.text_line(
            Face::Regular,
            BODY,
            CONTROL,
            22.0,
            qy,
            color::mix(color::MUTED, color::TEXT, hover),
            "Quit game",
        );
        ui.text(
            Face::Regular,
            SMALL,
            22.0,
            h - 12.0 - 18.0,
            color::FAINT,
            "Horizon 26.3",
        );
    }
    /// Home: the launcher's Play page over the backdrop.
    fn home(&self, ui: &mut Ui, x: f32, w: f32) {
        let mut y = 36.0 + 28.0;
        ui.text(
            Face::Medium,
            SMALL,
            x,
            y,
            color::FAINT,
            "Minecraft 26.3  ·  Java Edition, rewritten in Rust",
        );
        y += 22.0;
        ui.text_line(Face::Display, t::HERO, 58.0, x, y, color::TEXT, "Horizon");
        y += 58.0 + 12.0;
        ui.text(
            Face::Regular,
            t::LARGE,
            x,
            y,
            color::MUTED,
            "The world you know, rebuilt from the ground up.",
        );
        y += 22.0 + 24.0;
        let pw = ui.button_width("Singleplayer", true).max(120.0);
        ui.button(
            PLAY,
            [x, y, pw, CONTROL_LARGE],
            "Singleplayer",
            Kind::Primary,
            true,
        );
        let mw = ui.button_width("Multiplayer", true);
        ui.button(
            MULTIPLAYER,
            [x + pw + 8.0, y, mw, CONTROL_LARGE],
            "Multiplayer",
            Kind::Secondary,
            true,
        );
        y += CONTROL_LARGE + 24.0;
        ui.divider(x, y, w);
        let worlds = self.recent_worlds();
        let last = worlds.first();
        let value = last.map_or("None yet".to_owned(), |l| {
            let secs = l
                .modified
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            format!("{}  ·  {}", l.name, crate::title::date_label(secs))
        });
        let rh = ui.row(x, y, "Last played", &value, w - t::ROW_LABEL - 120.0);
        if last.is_some() {
            let bw = ui.button_width("Resume", false);
            ui.button(
                RESUME,
                [x + w - bw, y + (rh - CONTROL) / 2.0, bw, CONTROL],
                "Resume",
                Kind::Secondary,
                true,
            );
        }
        y += rh;
        ui.divider(x, y, w);
        let count = match worlds.len() {
            1 => "1 world".to_owned(),
            n => format!("{n} worlds"),
        };
        y += ui.row(x, y, "Saves", &count, w - t::ROW_LABEL - 16.0);
        ui.divider(x, y, w);
        let note = "Not affiliated with Mojang or Microsoft";
        ui.text(
            Face::Regular,
            SMALL,
            x,
            ui.h - 12.0 - 18.0,
            color::FAINT,
            note,
        );
    }
    /// A click in Horizon's layout (`x`, `y` in GUI pixels).
    pub fn click_horizon(
        &mut self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
    ) -> Action {
        if self.skip_opening() {
            return Action::None;
        }
        let hit = {
            let mut ui = Ui::dry(atlas, batch, (x, y));
            self.draw_horizon(&mut ui, 1.0);
            ui.hit((x, y))
        };
        let Some(hit) = hit else {
            if let Mode::Create { focus, .. } = &mut self.mode {
                *focus = 2;
            }
            return Action::None;
        };
        let saves = self.saves.clone();
        let select = |saves: &std::path::Path| Mode::Select {
            worlds: list_worlds(saves),
            selected: None,
        };
        match hit {
            QUIT => return Action::Quit,
            h if h == NAV => {
                self.mode = Mode::Main;
                return Action::None;
            }
            h if h == NAV + 1 => {
                if !matches!(self.mode, Mode::Select { .. }) {
                    self.mode = select(&saves);
                }
                return Action::None;
            }
            h if h == NAV + 2 => {
                if !matches!(self.mode, Mode::Multiplayer(_)) {
                    self.mode = Mode::Multiplayer(Screen::list(&saves));
                }
                return Action::None;
            }
            h if h == NAV + 3 => return Action::Options,
            _ => {}
        }
        match &mut self.mode {
            Mode::Main => match hit {
                PLAY => self.mode = select(&saves),
                MULTIPLAYER => self.mode = Mode::Multiplayer(Screen::list(&saves)),
                RESUME => {
                    if let Some(w) = list_worlds(&saves).into_iter().next() {
                        return Action::Play { folder: w.folder };
                    }
                }
                _ => {}
            },
            Mode::Select { worlds, selected } => match hit {
                PLAY_WORLD => {
                    if let Some(w) = selected.and_then(|i| worlds.get(i)) {
                        return Action::Play {
                            folder: w.folder.clone(),
                        };
                    }
                }
                CREATE => {
                    self.mode = Mode::Create {
                        name: "New World".into(),
                        seed: String::new(),
                        creative: false,
                        focus: 0,
                    }
                }
                h if (WORLD..WORLD + 10_000).contains(&h) => {
                    let i = (h - WORLD) as usize;
                    if *selected == Some(i) {
                        return Action::Play {
                            folder: worlds[i].folder.clone(),
                        };
                    }
                    *selected = Some(i);
                }
                _ => {}
            },
            Mode::Create {
                creative, focus, ..
            } => match hit {
                BACK => self.mode = select(&saves),
                NAME => *focus = 0,
                SEED => *focus = 1,
                h if h == MODE || h == MODE + 1 => *creative = h == MODE + 1,
                MAKE => return self.create_action(),
                _ => {}
            },
            Mode::Multiplayer(screen) => {
                let id = match hit {
                    JOIN => Some(40),
                    DIRECT => Some(41),
                    ADD => Some(42),
                    EDIT => Some(43),
                    DELETE => Some(44),
                    REFRESH => Some(45),
                    BACK => Some(match screen {
                        Screen::List { .. } => 46,
                        Screen::Edit { .. } => 51,
                        Screen::Direct { .. } => 53,
                        Screen::Disconnected { .. } => 54,
                    }),
                    CANCEL_RETRY => Some(55),
                    DONE => Some(match screen {
                        Screen::Direct { .. } => 52,
                        _ => 50,
                    }),
                    FIELD_A | FIELD_B => {
                        if let Screen::Edit { focus, .. } = screen {
                            *focus = u8::from(hit == FIELD_B);
                        }
                        None
                    }
                    h if (SERVER..SERVER + 10_000).contains(&h) => {
                        let i = (h - SERVER) as usize;
                        let again =
                            matches!(screen, Screen::List { selected, .. } if *selected == Some(i));
                        if again {
                            if let Some(address) = screen.address_of(i) {
                                return Action::Connect { address };
                            }
                        } else if let Screen::List { selected, .. } = screen {
                            *selected = Some(i);
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(id) = id {
                    match screen.click(Some(id), 0.0, 0.0, (0.0, 0.0), &saves) {
                        Outcome::Back => self.mode = Mode::Main,
                        Outcome::Connect(address) => return Action::Connect { address },
                        Outcome::None => {}
                    }
                }
            }
        }
        Action::None
    }
}

/// The page header plus a row of actions under it; returns the y below.
fn header(ui: &mut Ui, x: f32, title: &str, description: &str) -> f32 {
    let y = 36.0;
    y + ui.page_header(x, y, title, description) + 24.0
}
/// Actions laid out right to left from `right`; returns the left edge.
fn actions_right(ui: &mut Ui, right: f32, y: f32, items: &[(u64, &str, Kind, bool)]) -> f32 {
    let mut rx = right;
    for &(bid, label, kind, on) in items {
        let large = kind == Kind::Primary;
        let (bw, bh) = if large {
            (ui.button_width(label, true).max(120.0), CONTROL_LARGE)
        } else {
            (ui.button_width(label, false), CONTROL)
        };
        rx -= bw;
        ui.button(
            bid,
            [rx, y + (CONTROL_LARGE - bh) / 2.0, bw, bh],
            label,
            kind,
            on,
        );
        rx -= 8.0;
    }
    rx
}
/// A selectable list row: the launcher's row look with a hover layer and
/// the raised selection with its 1px border. Returns the row height.
#[allow(clippy::too_many_arguments)]
fn list_row(
    ui: &mut Ui,
    rid: u64,
    x: f32,
    y: f32,
    w: f32,
    selected: bool,
    title: &str,
    meta: &str,
    right: &str,
    dot: Option<u32>,
) -> f32 {
    let h = 60.0;
    let hover = ui.interact(rid, x - 12.0, y + 2.0, w + 24.0, h - 4.0, true);
    let sel = ui.tween(rid ^ 0x5e1ec7, if selected { 1.0 } else { 0.0 }, t::quick());
    ui.rounded(
        x - 12.0,
        y + 4.0,
        w + 24.0,
        h - 8.0,
        t::RADIUS,
        color::with_alpha(color::HOVER, 0.7 * hover),
    );
    ui.rounded(
        x - 12.0,
        y + 4.0,
        w + 24.0,
        h - 8.0,
        t::RADIUS,
        color::with_alpha(color::RAISED, sel),
    );
    ui.border(
        x - 12.0,
        y + 4.0,
        w + 24.0,
        h - 8.0,
        t::RADIUS,
        color::with_alpha(color::BORDER, sel),
    );
    let mut tx = x;
    if let Some(dot) = dot {
        ui.rounded(x, y + 17.0, 6.0, 6.0, 3.0, dot);
        tx += 16.0;
    }
    let title = ui.fit(Face::Medium, BODY, title, w - 200.0);
    ui.text(Face::Medium, BODY, tx, y + 10.0, color::TEXT, &title);
    let meta = ui.fit(Face::Regular, SMALL, meta, w - 200.0);
    ui.text(Face::Regular, SMALL, tx, y + 31.0, color::MUTED, &meta);
    ui.text_right(Face::Regular, SMALL, x + w, y + 21.0, color::FAINT, right);
    ui.divider(x, y + h, w);
    h
}
/// An empty list: a serif line and a hint.
fn empty(ui: &mut Ui, x: f32, y: f32, title: &str, hint: &str) {
    ui.text_line(Face::Display, 22.0, 30.0, x, y + 32.0, color::TEXT, title);
    ui.text(Face::Regular, BODY, x, y + 66.0, color::MUTED, hint);
}

fn worlds_page(
    ui: &mut Ui,
    x: f32,
    w: f32,
    worlds: &[crate::title::WorldEntry],
    selected: Option<usize>,
) {
    let count = match worlds.len() {
        0 => "No worlds yet. They live in the saves folder.".to_owned(),
        1 => "1 world in the saves folder.".to_owned(),
        n => format!("{n} worlds in the saves folder."),
    };
    let mut y = header(ui, x, "Singleplayer", &count);
    actions_right(
        ui,
        x + w,
        y - 24.0 - 58.0,
        &[
            (PLAY_WORLD, "Play", Kind::Primary, selected.is_some()),
            (CREATE, "Create world", Kind::Secondary, true),
        ],
    );
    ui.divider(x, y, w);
    if worlds.is_empty() {
        empty(ui, x, y, "A blank map.", "Create a world to begin.");
        return;
    }
    let bottom = ui.h - 36.0;
    for (i, world) in worlds.iter().enumerate() {
        if y + 60.0 > bottom {
            break;
        }
        let secs = world
            .modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let meta = format!("{}  ·  {}", crate::title::date_label(secs), world.folder);
        let mode = if world.creative {
            "Creative"
        } else {
            "Survival"
        };
        y += list_row(
            ui,
            WORLD + i as u64,
            x,
            y,
            w,
            selected == Some(i),
            &world.name,
            &meta,
            mode,
            None,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn create_page(
    ui: &mut Ui,
    x: f32,
    w: f32,
    name: &str,
    seed: &str,
    creative: bool,
    focus: u8,
    caret: bool,
) {
    let mut y = header(
        ui,
        x,
        "Create world",
        "Generated by Horizon from the seed, as vanilla would.",
    );
    actions_right(
        ui,
        x + w,
        y - 24.0 - 58.0,
        &[
            (MAKE, "Create", Kind::Primary, true),
            (BACK, "Cancel", Kind::Ghost, true),
        ],
    );
    ui.divider(x, y, w);
    let fx = x + t::ROW_LABEL + 16.0;
    let fw = (w - t::ROW_LABEL - 16.0).min(360.0);
    for (i, label) in ["World name", "Seed", "Game mode"].iter().enumerate() {
        let rh = 12.0 + CONTROL + 12.0;
        ui.text_line(Face::Regular, BODY, rh, x, y, color::MUTED, label);
        let r = [fx, y + 12.0, fw, CONTROL];
        match i {
            0 => ui.field(NAME, r, name, "New World", focus == 0, caret),
            1 => ui.field(SEED, r, seed, "Random", focus == 1, caret),
            _ => {
                ui.segmented(
                    MODE,
                    fx,
                    y + 12.0,
                    &["Survival", "Creative"],
                    usize::from(creative),
                );
            }
        }
        y += rh;
        ui.divider(x, y, w);
    }
}

fn multiplayer_page(ui: &mut Ui, x: f32, w: f32, screen: &Screen, caret: bool) {
    match screen {
        Screen::List {
            servers,
            selected,
            pings,
            lan,
        } => {
            let games = lan.games();
            let sub = match servers.len() {
                0 => "No saved servers. Worlds opened to LAN show up here too.".to_owned(),
                1 => "1 saved server, and worlds opened to LAN.".to_owned(),
                n => format!("{n} saved servers, and worlds opened to LAN."),
            };
            let mut y = header(ui, x, "Multiplayer", &sub);
            let has = selected.is_some();
            let saved = selected.is_some_and(|i| i < servers.len());
            let mut items = vec![
                (JOIN, "Join", Kind::Primary, has),
                (ADD, "Add server", Kind::Secondary, true),
                (DIRECT, "Direct connect", Kind::Secondary, true),
                (REFRESH, "Refresh", Kind::Ghost, true),
            ];
            if saved {
                items.push((EDIT, "Edit", Kind::Ghost, true));
                items.push((DELETE, "Delete", Kind::Danger, true));
            }
            actions_right(ui, x + w, y - 24.0 - 58.0, &items);
            ui.divider(x, y, w);
            let pings = pings.lock().map(|p| p.clone()).unwrap_or_default();
            let mut rows: Vec<(String, String, String, u32)> = servers
                .iter()
                .map(|s| {
                    let (line, right, dot) = match pings.get(&s.ip) {
                        Some(Ping::Ok {
                            motd, players, ms, ..
                        }) => (
                            motd.lines().next().unwrap_or("").trim().to_owned(),
                            format!("{players}  ·  {ms} ms"),
                            color::ACCENT,
                        ),
                        Some(Ping::Failed(_)) => (
                            "Can't reach the server".to_owned(),
                            String::new(),
                            color::DANGER,
                        ),
                        _ => ("Pinging…".to_owned(), String::new(), color::FAINT),
                    };
                    (s.name.clone(), format!("{line}  ·  {}", s.ip), right, dot)
                })
                .collect();
            rows.extend(games.iter().map(|g| {
                (
                    "LAN world".to_owned(),
                    format!("{}  ·  {}", g.motd, g.address),
                    "On your network".to_owned(),
                    color::ACCENT,
                )
            }));
            if rows.is_empty() {
                empty(
                    ui,
                    x,
                    y,
                    "Nobody here yet.",
                    "Add a server, connect directly, or open a world to LAN.",
                );
                return;
            }
            let bottom = ui.h - 36.0;
            for (i, (name, meta, right, dot)) in rows.iter().enumerate() {
                if y + 60.0 > bottom {
                    break;
                }
                y += list_row(
                    ui,
                    SERVER + i as u64,
                    x,
                    y,
                    w,
                    *selected == Some(i),
                    name,
                    meta,
                    right,
                    Some(*dot),
                );
            }
        }
        Screen::Edit {
            index,
            name,
            address,
            focus,
        } => {
            let title = if index.is_some() {
                "Edit server"
            } else {
                "Add server"
            };
            let mut y = header(
                ui,
                x,
                title,
                "Saved to servers.dat, like vanilla's server list.",
            );
            let ok = horizon_protocol::servers::parse_address(address).is_some();
            actions_right(
                ui,
                x + w,
                y - 24.0 - 58.0,
                &[
                    (DONE, "Save", Kind::Primary, ok),
                    (BACK, "Cancel", Kind::Ghost, true),
                ],
            );
            ui.divider(x, y, w);
            let fx = x + t::ROW_LABEL + 16.0;
            let fw = (w - t::ROW_LABEL - 16.0).min(360.0);
            for (i, (label, value, hint)) in [
                ("Name", name, "Minecraft Server"),
                ("Address", address, "play.example.net"),
            ]
            .iter()
            .enumerate()
            {
                let rh = 12.0 + CONTROL + 12.0;
                ui.text_line(Face::Regular, BODY, rh, x, y, color::MUTED, label);
                let fid = if i == 0 { FIELD_A } else { FIELD_B };
                ui.field(
                    fid,
                    [fx, y + 12.0, fw, CONTROL],
                    value,
                    hint,
                    usize::from(*focus) == i,
                    caret,
                );
                y += rh;
                ui.divider(x, y, w);
            }
        }
        Screen::Direct { address } => {
            let y = header(
                ui,
                x,
                "Direct connect",
                "Join a server once without saving it.",
            );
            let ok = horizon_protocol::servers::parse_address(address).is_some();
            actions_right(
                ui,
                x + w,
                y - 24.0 - 58.0,
                &[
                    (DONE, "Join", Kind::Primary, ok),
                    (BACK, "Cancel", Kind::Ghost, true),
                ],
            );
            ui.divider(x, y, w);
            let rh = 12.0 + CONTROL + 12.0;
            ui.text_line(Face::Regular, BODY, rh, x, y, color::MUTED, "Address");
            let fx = x + t::ROW_LABEL + 16.0;
            ui.field(
                FIELD_A,
                [fx, y + 12.0, (w - t::ROW_LABEL - 16.0).min(360.0), CONTROL],
                address,
                "host:port",
                true,
                caret,
            );
            ui.divider(x, y + rh, w);
        }
        Screen::Disconnected { reason, retry } => {
            let mut y = header(
                ui,
                x,
                "Connection lost",
                "The server closed the connection.",
            );
            ui.divider(x, y, w);
            y += 16.0;
            // The launcher's error note: a danger dot beside the message.
            let lines = wrap(ui, reason, w - 40.0);
            let box_h = 24.0 + lines.len() as f32 * 19.0;
            ui.rounded(x, y, w, box_h, t::RADIUS, color::RAISED);
            ui.border(x, y, w, box_h, t::RADIUS, color::BORDER);
            ui.rounded(x + 12.0, y + 18.0, 6.0, 6.0, 3.0, color::DANGER);
            for (i, line) in lines.iter().enumerate() {
                ui.text_line(
                    Face::Regular,
                    BODY,
                    19.0,
                    x + 28.0,
                    y + 12.0 + i as f32 * 19.0,
                    color::TEXT,
                    line,
                );
            }
            y += box_h + 16.0;
            if let Some(r) = retry {
                // Auto-reconnect's countdown, with a way out.
                let text = r.text(std::time::Instant::now());
                ui.text_line(Face::Regular, BODY, 19.0, x, y, color::MUTED, &text);
                y += 19.0 + 12.0;
            }
            let bw = ui.button_width("Back to servers", false);
            ui.button(
                BACK,
                [x, y, bw, CONTROL],
                "Back to servers",
                Kind::Secondary,
                true,
            );
            if retry.is_some() {
                let cw = ui.button_width("Cancel reconnecting", false);
                ui.button(
                    CANCEL_RETRY,
                    [x + bw + 8.0, y, cw, CONTROL],
                    "Cancel reconnecting",
                    Kind::Ghost,
                    true,
                );
            }
        }
    }
}
/// Greedy word wrap in dp.
pub fn wrap(ui: &Ui, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let next = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && ui.measure(Face::Regular, BODY, &next) > width {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = next;
            }
        }
        lines.push(line);
    }
    lines
}

/// The pause menu in Horizon's style: the launcher's sidebar over the
/// blurred world, with the game's actions as a page. Ids match
/// `game/screens.rs`'s PauseScreen actions: 0 back to game, 1 options,
/// 2 save and quit, 3 statistics, 4 advancements, 9 world options.
pub mod pause {
    use super::*;
    fn act(a: u8) -> u64 {
        id("pause", u64::from(a))
    }
    pub fn draw(ui: &mut Ui, world: &str, quit_label: &str, opened: Instant) {
        let (w, h) = (ui.w, ui.h);
        let p = t::ease_out_quint(opened.elapsed().as_secs_f32() / t::move_());
        ui.opacity = p;
        ui.atmosphere(0.35, 1.0);
        ui.offset = (-16.0 * (1.0 - p), 0.0);
        ui.fill(0.0, 0.0, SIDEBAR, h, color::with_alpha(color::SIDEBAR, 0.8));
        ui.vline(SIDEBAR - 1.0, 0.0, h, color::BORDER);
        ui.text_line(
            Face::Display,
            22.0,
            30.0,
            22.0,
            16.0,
            color::TEXT,
            "Horizon",
        );
        ui.sidebar_list(
            id("pause.nav", 0),
            12.0,
            66.0,
            SIDEBAR - 24.0,
            &["Game", "Advancements", "Statistics", "Options"],
            0,
        );
        ui.text(
            Face::Regular,
            SMALL,
            22.0,
            h - 12.0 - 18.0,
            color::FAINT,
            "Horizon 26.3",
        );
        ui.offset = (0.0, 8.0 * (1.0 - p));
        let x = SIDEBAR + 48.0;
        let cw = (w - x - 48.0).min(760.0);
        let mut y = 36.0;
        ui.text(Face::Medium, SMALL, x, y, color::FAINT, "Game paused");
        y += 22.0;
        let title = ui.fit(Face::Display, t::HERO, world, cw);
        ui.text_line(Face::Display, t::HERO, 58.0, x, y, color::TEXT, &title);
        y += 58.0 + 24.0;
        let bw = ui.button_width("Back to game", true).max(120.0);
        ui.button(
            act(0),
            [x, y, bw, CONTROL_LARGE],
            "Back to game",
            Kind::Primary,
            true,
        );
        let lw = ui.button_width("Open to LAN", true);
        ui.button(
            act(9),
            [x + bw + 8.0, y, lw, CONTROL_LARGE],
            "Open to LAN",
            Kind::Secondary,
            true,
        );
        y += CONTROL_LARGE + 24.0;
        ui.divider(x, y, cw);
        let rh = ui.row(
            x,
            y,
            "Leave",
            "Saves the world, then returns to the title.",
            cw - t::ROW_LABEL - 220.0,
        );
        let qw = ui.button_width(quit_label, false);
        ui.button(
            act(2),
            [x + cw - qw, y + (rh - CONTROL) / 2.0, qw, CONTROL],
            quit_label,
            Kind::Danger,
            true,
        );
        y += rh;
        ui.divider(x, y, cw);
        ui.opacity = 1.0;
        ui.offset = (0.0, 0.0);
    }
    /// The PauseScreen action under a click, if any.
    pub fn click(
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        world: &str,
        quit_label: &str,
    ) -> Option<u8> {
        let mut ui = Ui::dry(atlas, batch, (x, y));
        draw(
            &mut ui,
            world,
            quit_label,
            Instant::now() - std::time::Duration::from_secs(5),
        );
        let hit = ui.hit((x, y))?;
        let nav = id("pause.nav", 0);
        match hit {
            h if h == nav => Some(0),
            h if h == nav + 1 => Some(4),
            h if h == nav + 2 => Some(3),
            h if h == nav + 3 => Some(1),
            h => [0u8, 1, 2, 3, 4, 9].into_iter().find(|&a| act(a) == h),
        }
    }
}
