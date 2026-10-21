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
const DELETE_WORLD: u64 = id("worlds.delete", 0);
const CONFIRM_DELETE: u64 = id("delete.confirm", 0);
const NAME: u64 = id("create.name", 0);
const SEED: u64 = id("create.seed", 0);
const MODE: u64 = id("create.mode", 0);
const MAKE: u64 = id("create.make", 0);
const SERVER: u64 = id("mp.row", 0);
const JOIN: u64 = id("mp.join", 0);
const DIRECT: u64 = id("mp.direct", 0);
const FRIEND: u64 = id("mp.friend", 0);
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
        Mode::Select { .. }
        | Mode::Create { .. }
        | Mode::Edit { .. }
        | Mode::Backup { .. }
        | Mode::Delete { .. }
        | Mode::Error { .. } => 1,
        Mode::Multiplayer(_) => 2,
    }
}
const PAGES: [&str; 4] = ["Home", "Singleplayer", "Multiplayer", "Options"];
/// World editing and backups still use the classic screens, and so do their
/// errors; the rest of the singleplayer flow has Horizon pages.
fn classic(mode: &Mode) -> bool {
    match mode {
        Mode::Edit { .. } | Mode::Backup { .. } => true,
        Mode::Error { return_to, .. } => classic(return_to),
        _ => false,
    }
}

impl Title {
    /// Horizon's layout for every title mode. `backdrop` is how much of the
    /// live backdrop is visible (0 shows the plain cold background).
    pub fn draw_horizon(&self, ui: &mut Ui, backdrop: f32) {
        if classic(&self.mode) {
            self.draw(ui.atlas, ui.batch, ui.cursor, false);
            return;
        }
        let reveal = self.opening_phase(backdrop >= 1.0);
        if backdrop < 1.0 {
            ui.cold_background(1.0 - backdrop);
        }
        let shade = t::ease_out_cubic(reveal);
        // The world stays in view on the home page; deeper pages veil it more.
        let veil = if matches!(self.mode, Mode::Main) {
            horizon_theme::HOME_VEIL
        } else {
            horizon_theme::VEIL
        };
        ui.muted = if matches!(self.mode, Mode::Main) {
            color::MUTED
        } else {
            color::FAINT
        };
        ui.atmosphere(veil, shade);
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
                    ..
                } => {
                    let caret = (self.started.elapsed().as_millis() / 500).is_multiple_of(2);
                    create_page(ui, x, w, name, seed, *creative, *focus, caret);
                }
                Mode::Delete { world } => delete_page(ui, x, w, world),
                Mode::Error { message, .. } => error_page(ui, x, w, message),
                Mode::Multiplayer(screen) => {
                    let caret = (self.started.elapsed().as_millis() / 500).is_multiple_of(2);
                    multiplayer_page(ui, x, w, screen, caret);
                }
                Mode::Edit { .. } | Mode::Backup { .. } => {
                    unreachable!("World editing uses the classic layout")
                }
            }
        }
        ui.opacity = 1.0;
        ui.offset = (0.0, 0.0);
        if reveal < 1.0 {
            // The wordmark rises and fades as the interface comes in.
            let word = 1.0 - t::ease_out_cubic(reveal / 0.6);
            ui.opacity = word;
            let tw = ui.measure(Face::Display, t::WORDMARK, "horizon");
            let line = (t::WORDMARK * 1.2).round();
            ui.text_line(
                Face::Display,
                t::WORDMARK,
                line,
                ((ui.w - tw) / 2.0).round(),
                (ui.h - line) / 2.0 - 28.0 * (1.0 - word),
                color::TEXT,
                "horizon",
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
        let (h, muted) = (ui.h, ui.muted);
        ui.sidebar_frame("Horizon");
        ui.sidebar_list(NAV, 12.0, 72.0, SIDEBAR - 24.0, &PAGES, page(&self.mode));
        let qy = h - 12.0 - 18.0 - 12.0 - CONTROL;
        let hover = ui.interact(QUIT, 12.0, qy, SIDEBAR - 24.0, CONTROL, true);
        ui.rounded(
            12.0,
            qy,
            SIDEBAR - 24.0,
            CONTROL,
            t::PILL,
            color::with_alpha(color::RAISED, 0.8 * hover),
        );
        ui.text_line(
            Face::Regular,
            BODY,
            CONTROL,
            24.0,
            qy,
            color::mix(muted, color::TEXT, hover),
            "Quit game",
        );
        ui.caps(
            24.0,
            h - 12.0 - 18.0,
            muted,
            &format!("Horizon {}", horizon_core::VANILLA_VERSION),
        );
    }
    /// Home: the launcher's Play page over the backdrop.
    fn home(&self, ui: &mut Ui, x: f32, w: f32) {
        ui.wordmark(x, w, ui.h - 64.0, 220.0);
        let muted = ui.muted;
        let mut y = 36.0 + 28.0;
        ui.caps(
            x,
            y,
            muted,
            &format!(
                "Minecraft {}  ·  Java Edition, rewritten in Rust",
                horizon_core::VANILLA_VERSION
            ),
        );
        y += 24.0;
        let line = (t::HERO * 1.1).round();
        ui.text_line(Face::Display, t::HERO, line, x, y, color::TEXT, "Horizon");
        y += line + 10.0;
        ui.text(
            Face::Regular,
            t::LARGE,
            x,
            y,
            muted,
            "The world you know, rebuilt from the ground up.",
        );
        y += 22.0 + 28.0;
        let pw = ui.button_width("Singleplayer", true).max(132.0);
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
        y += CONTROL_LARGE + 32.0;
        ui.row_index = 0;
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
        ui.caps(
            x,
            ui.h - 12.0 - 18.0,
            muted,
            "Not affiliated with Mojang or Microsoft",
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
        self.remember_direct_address();
        if self.skip_opening() {
            return Action::None;
        }
        if classic(&self.mode) {
            return self.click(x, y, (batch.width, batch.height)).0;
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
                CREATE => self.mode = Title::new_world(),
                DELETE_WORLD => {
                    if let Some(world) = selected.and_then(|i| worlds.get(i)).cloned() {
                        self.mode = Mode::Delete { world };
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
            Mode::Delete { .. } => match hit {
                // The selection stays in place: on the world that moved up
                // into the deleted one's row, or the new last one.
                CONFIRM_DELETE => {
                    if let Some(row) = self.confirm_delete()
                        && let Mode::Select { worlds, selected } = &mut self.mode
                    {
                        *selected = worlds.len().checked_sub(1).map(|last| row.min(last));
                    }
                }
                BACK => self.escape(),
                _ => {}
            },
            Mode::Error { return_to, .. } => {
                if hit == BACK {
                    self.mode = *return_to.clone();
                }
            }
            Mode::Edit { .. } | Mode::Backup { .. } => {}
            Mode::Multiplayer(screen) => {
                let id = match hit {
                    JOIN => Some(40),
                    DIRECT => Some(41),
                    FRIEND => Some(47),
                    ADD => Some(42),
                    EDIT => Some(43),
                    DELETE => Some(44),
                    REFRESH => Some(45),
                    BACK => Some(match screen {
                        Screen::List { .. } => 46,
                        Screen::Edit { .. } => 51,
                        Screen::Direct { .. } => 53,
                        Screen::Friend { .. } => 57,
                        Screen::Disconnected { .. } => 54,
                        Screen::ConfirmDelete { .. } => 59,
                    }),
                    CONFIRM_DELETE => Some(58),
                    CANCEL_RETRY => Some(55),
                    DONE => Some(match screen {
                        Screen::Direct { .. } => 52,
                        Screen::Friend { .. } => 56,
                        _ => 50,
                    }),
                    FIELD_A | FIELD_B => {
                        if let Screen::Edit { focus, .. } | Screen::Friend { focus, .. } = screen {
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
                    match self.multiplayer_click(Some(id), 0.0, 0.0, (0.0, 0.0)) {
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

/// The end of `text` that fits `width`, led by an ellipsis when cut.
fn tail_fit(ui: &Ui, text: &str, width: f32) -> String {
    if ui.measure(Face::Regular, BODY, text) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let tail: String = std::iter::once('…')
            .chain(chars[start..].iter().copied())
            .collect();
        if ui.measure(Face::Regular, BODY, &tail) <= width {
            return tail;
        }
        start += 1;
    }
    String::new()
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
/// A selectable list row: its number, the name and a line under it, a mono
/// detail on the right; a soft plate fades in under the pointer and stays
/// under the selection, reaching past the row. Returns the row height.
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
    let h = 64.0;
    let hover = ui.interact(rid, x - 16.0, y + 2.0, w + 32.0, h - 4.0, true);
    let sel = ui.tween(rid ^ 0x5e1ec7, if selected { 1.0 } else { 0.0 }, t::quick());
    ui.rounded(
        x - 16.0,
        y + 3.0,
        w + 32.0,
        h - 6.0,
        t::RADIUS_PLATE,
        color::with_alpha(color::RAISED, (0.7 * hover).max(sel)),
    );
    ui.row_index += 1;
    let n = format!("{:02}", ui.row_index);
    ui.text(
        Face::Mono,
        t::MICRO,
        x,
        y + 15.0,
        color::mix(color::FAINT, color::TEXT, sel),
        &n,
    );
    let mut tx = x + t::INDEX;
    if let Some(dot) = dot {
        ui.rounded(tx, y + 19.0, 6.0, 6.0, 3.0, dot);
        tx += 16.0;
    }
    let title = ui.fit(Face::Medium, t::LARGE + 2.0, title, w - 240.0);
    ui.text(
        Face::Medium,
        t::LARGE + 2.0,
        tx,
        y + 10.0,
        color::TEXT,
        &title,
    );
    let meta = ui.fit(Face::Regular, SMALL, meta, w - 240.0);
    ui.text(Face::Regular, SMALL, tx, y + 35.0, color::FAINT, &meta);
    let right = right.to_uppercase();
    ui.text_right(Face::Mono, t::MICRO, x + w, y + 25.0, color::FAINT, &right);
    ui.divider(x, y + h, w);
    h
}
/// An empty list: a short line in heading type and a hint.
fn empty(ui: &mut Ui, x: f32, y: f32, title: &str, hint: &str) {
    ui.text_line(
        Face::Display,
        t::HEADING,
        28.0,
        x,
        y + 32.0,
        color::TEXT,
        title,
    );
    ui.text(Face::Regular, BODY, x, y + 64.0, color::FAINT, hint);
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
    let mut items = vec![
        (PLAY_WORLD, "Play", Kind::Primary, selected.is_some()),
        (CREATE, "Create world", Kind::Secondary, true),
    ];
    if selected.is_some() {
        items.push((DELETE_WORLD, "Delete", Kind::Danger, true));
    }
    actions_right(ui, x + w, y - 24.0 - 58.0, &items);
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

/// The launcher's notice: a plate led by a danger dot, the first line in
/// medium type, wrapped lines under it. Returns its height.
fn notice(ui: &mut Ui, x: f32, y: f32, w: f32, title: Option<&str>, text: &str) -> f32 {
    let lines = wrap(ui, text, w - 48.0);
    let title_h = if title.is_some() { 26.0 } else { 0.0 };
    let h = 32.0 + title_h + lines.len() as f32 * 19.0;
    ui.rounded(x, y, w, h, t::RADIUS_PLATE, color::RAISED);
    ui.rounded(x + 16.0, y + 22.0, 6.0, 6.0, 3.0, color::DANGER);
    let mut ty = y + 16.0;
    if let Some(title) = title {
        let title = ui.fit(Face::Medium, t::LARGE + 2.0, title, w - 48.0);
        ui.text_line(
            Face::Medium,
            t::LARGE + 2.0,
            19.0,
            x + 32.0,
            ty,
            color::TEXT,
            &title,
        );
        ty += title_h;
    }
    for (i, line) in lines.iter().enumerate() {
        let tint = if title.is_some() {
            color::MUTED
        } else {
            color::TEXT
        };
        ui.text_line(
            Face::Regular,
            BODY,
            19.0,
            x + 32.0,
            ty + i as f32 * 19.0,
            tint,
            line,
        );
    }
    h
}

/// `ConfirmScreen` for a world: what goes, that it can't come back, then
/// Delete (danger) and Cancel under it.
fn delete_page(ui: &mut Ui, x: f32, w: f32, world: &crate::title::WorldEntry) {
    let mut y = header(
        ui,
        x,
        "Delete this world?",
        "It will be lost forever. This can't be undone.",
    );
    ui.divider(x, y, w);
    y += 16.0;
    let secs = world
        .modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let text = format!(
        "Last played {}. The folder saves/{} and everything built in it will be removed.",
        crate::title::date_label(secs),
        world.folder
    );
    y += notice(ui, x, y, w, Some(&world.name), &text) + 16.0;
    let dw = ui.button_width("Delete world", false);
    ui.button(
        CONFIRM_DELETE,
        [x, y, dw, CONTROL],
        "Delete world",
        Kind::Danger,
        true,
    );
    let cw = ui.button_width("Cancel", false);
    ui.button(
        BACK,
        [x + dw + 8.0, y, cw, CONTROL],
        "Cancel",
        Kind::Ghost,
        true,
    );
}

fn error_page(ui: &mut Ui, x: f32, w: f32, message: &str) {
    let mut y = header(
        ui,
        x,
        "Something went wrong",
        "Horizon couldn't finish that.",
    );
    ui.divider(x, y, w);
    y += 16.0;
    y += notice(ui, x, y, w, None, message) + 16.0;
    let bw = ui.button_width("Back", false);
    ui.button(BACK, [x, y, bw, CONTROL], "Back", Kind::Secondary, true);
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
            ..
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
                (FRIEND, "Join a friend", Kind::Secondary, true),
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
                            motd.resolve_legacy()
                                .plain()
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_owned(),
                            format!("{}  ·  {ms} ms", Ping::player_count(*players).plain()),
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
        Screen::Friend { code, relay, focus } => {
            let y = header(
                ui,
                x,
                "Join a friend",
                "Paste the invite code your friend sees in their pause menu.",
            );
            let parsed = horizon_friends::invite::parse_detailed(code);
            let ok = parsed.is_ok();
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
            let fx = x + t::ROW_LABEL + 16.0;
            let fw = w - t::ROW_LABEL - 16.0;
            let rows = [
                (FIELD_A, "Invite code", code, "Paste the code here", fw),
                (
                    FIELD_B,
                    "Relay (optional)",
                    relay,
                    "host:port, for short codes",
                    fw.min(360.0),
                ),
            ];
            let mut ry = y;
            for (i, (fid, label, value, hint, width)) in rows.into_iter().enumerate() {
                ui.text_line(Face::Regular, BODY, rh, x, ry, color::MUTED, label);
                // A long code shows its end, where the typing is.
                let shown = tail_fit(ui, value, width - 28.0);
                ui.field(
                    fid,
                    [fx, ry + 12.0, width, CONTROL],
                    &shown,
                    hint,
                    usize::from(*focus) == i,
                    caret,
                );
                ry += rh;
                ui.divider(x, ry, w);
            }
            let note = match parsed {
                _ if code.trim().is_empty() => None,
                Err(e) => Some(e.to_string()),
                Ok(horizon_friends::invite::Parsed::Short(_)) if relay.trim().is_empty() => Some(
                    "This is a relay's short code: fill in your friend's relay above, or ask for the full code."
                        .to_owned(),
                ),
                Ok(_) => None,
            };
            if let Some(note) = note {
                ui.text(Face::Regular, SMALL, x, ry + 12.0, color::MUTED, &note);
            }
        }
        Screen::ConfirmDelete { name, .. } => {
            let mut y = header(
                ui,
                x,
                "Remove this server?",
                "It leaves your list. You can add it again later.",
            );
            ui.divider(x, y, w);
            y += 16.0;
            y += notice(
                ui,
                x,
                y,
                w,
                Some(name),
                "Its address and icon will be forgotten.",
            ) + 16.0;
            let dw = ui.button_width("Delete server", false);
            ui.button(
                CONFIRM_DELETE,
                [x, y, dw, CONTROL],
                "Delete server",
                Kind::Danger,
                true,
            );
            let cw = ui.button_width("Cancel", false);
            ui.button(
                BACK,
                [x + dw + 8.0, y, cw, CONTROL],
                "Cancel",
                Kind::Ghost,
                true,
            );
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
            y += notice(ui, x, y, w, None, reason) + 16.0;
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
/// Play with friends as the pause menu shows it (game/friends.rs fills it).
#[derive(Clone, Debug, Default)]
pub struct FriendsView {
    /// A singleplayer world (not a server): the button is offered.
    pub shown: bool,
    pub sharing: bool,
    pub code: Option<String>,
    /// Who can join and what to do when friends on the internet can't (or,
    /// before the code, what the game is doing).
    pub status: String,
    /// Friends on the internet can reach the host.
    pub reachable: bool,
    /// Where the troubleshooting is, when they can't.
    pub help: Option<String>,
    /// The players in the world besides the host.
    pub guests: Vec<String>,
    pub notice: Option<String>,
    /// The code was just copied.
    pub copied: bool,
}

pub mod pause {
    use super::*;
    fn act(a: u8) -> u64 {
        id("pause", u64::from(a))
    }
    pub fn draw(
        ui: &mut Ui,
        world: &str,
        quit_label: &str,
        opened: Instant,
        friends: &FriendsView,
        remote: bool,
    ) {
        let (w, h) = (ui.w, ui.h);
        let p = t::ease_out_quint(opened.elapsed().as_secs_f32() / t::move_());
        ui.opacity = p;
        ui.atmosphere(horizon_theme::VEIL, 1.0);
        ui.offset = (-16.0 * (1.0 - p), 0.0);
        ui.sidebar_frame("Horizon");
        ui.sidebar_list(
            id("pause.nav", 0),
            12.0,
            72.0,
            SIDEBAR - 24.0,
            &["Game", "Advancements", "Statistics", "Options"],
            0,
        );
        ui.caps(
            24.0,
            h - 12.0 - 18.0,
            color::FAINT,
            &format!("Horizon {}", horizon_core::VANILLA_VERSION),
        );
        ui.offset = (0.0, 8.0 * (1.0 - p));
        let x = SIDEBAR + 48.0;
        let cw = (w - x - 48.0).min(760.0);
        let mut y = 36.0 + 28.0;
        ui.caps(x, y, color::FAINT, "Game paused");
        y += 24.0;
        let line = (t::HERO * 1.1).round();
        let title = ui.fit(Face::Display, t::HERO, world, cw);
        ui.text_line(Face::Display, t::HERO, line, x, y, color::TEXT, &title);
        y += line + 32.0;
        let bw = ui.button_width("Back to game", true).max(120.0);
        ui.button(
            act(0),
            [x, y, bw, CONTROL_LARGE],
            "Back to game",
            Kind::Primary,
            true,
        );
        let lw = ui.button_width("World Options", true);
        ui.button(
            act(9),
            [x + bw + 8.0, y, lw, CONTROL_LARGE],
            "World Options",
            Kind::Secondary,
            true,
        );
        if friends.shown {
            let iw = ui.button_width("Invite friends", true);
            ui.button(
                act(10),
                [x + bw + lw + 16.0, y, iw, CONTROL_LARGE],
                "Invite friends",
                Kind::Secondary,
                !friends.sharing,
            );
        }
        y += CONTROL_LARGE + 32.0;
        ui.row_index = 0;
        if friends.sharing {
            y = invite(ui, x, y, cw, friends);
        }
        ui.divider(x, y, cw);
        let rh = ui.row(
            x,
            y,
            "Leave",
            if remote {
                "Disconnects from the server, then returns to the title."
            } else {
                "Saves the world, then returns to the title."
            },
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
    /// The invite section while sharing: the code with Copy and Stop
    /// sharing, who is here, and what changed. Returns the y below it.
    /// The code's groups packed into lines of at most `width`.
    fn code_lines(ui: &Ui, code: &str, width: f32) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for group in code.split('-') {
            match lines.last_mut() {
                Some(l) if ui.measure(Face::Mono, BODY, &format!("{l}-{group}")) <= width => {
                    l.push('-');
                    l.push_str(group);
                }
                _ => lines.push(group.to_owned()),
            }
        }
        lines
    }
    fn invite(ui: &mut Ui, x: f32, mut y: f32, cw: f32, f: &FriendsView) -> f32 {
        ui.divider(x, y, cw);
        let top = y + 12.0;
        ui.text(
            Face::Regular,
            BODY,
            x,
            top + 6.0,
            color::MUTED,
            "Invite code",
        );
        let vx = x + t::ROW_LABEL + 16.0;
        let stop = "Stop sharing";
        let sw = ui.button_width(stop, false);
        ui.button(
            act(12),
            [x + cw - sw, top, sw, CONTROL],
            stop,
            Kind::Danger,
            true,
        );
        let copy = if f.copied { "Copied" } else { "Copy" };
        let copy_w = ui.button_width("Copied", false);
        let buttons = sw + 8.0 + copy_w;
        ui.button(
            act(11),
            [x + cw - buttons, top, copy_w, CONTROL],
            copy,
            Kind::Secondary,
            f.code.is_some(),
        );
        let line = (BODY * 1.45).round();
        let value_w = (cw - t::ROW_LABEL - 16.0 - buttons - 16.0).max(120.0);
        y = top + 6.0;
        match &f.code {
            Some(code) => {
                for l in code_lines(ui, code, value_w) {
                    ui.text(Face::Mono, BODY, vx, y, color::TEXT, &l);
                    y += line;
                }
            }
            None => {
                for l in wrap(ui, &f.status, value_w) {
                    ui.text(Face::Regular, BODY, vx, y, color::MUTED, &l);
                    y += line;
                }
            }
        }
        y = y.max(top + CONTROL) + 10.0;
        let small = (SMALL * 1.45).round();
        if f.code.is_some() {
            // Who can join: plain when friends anywhere can, in the text
            // colour (it asks for something) when only the LAN and VPNs can.
            let tint = if f.reachable {
                color::MUTED
            } else {
                color::TEXT
            };
            for l in wrap(ui, &f.status, cw) {
                ui.text(Face::Regular, SMALL, x, y, tint, &l);
                y += small;
            }
            if let Some(help) = &f.help {
                let l = ui.fit(Face::Regular, SMALL, &format!("Help: {help}"), cw);
                ui.text(Face::Regular, SMALL, x, y, color::FAINT, &l);
                y += small;
            }
        }
        let here = if f.guests.is_empty() {
            "No friends here yet.".to_owned()
        } else {
            format!("Here now: {}", f.guests.join(", "))
        };
        let hint =
            "Friends paste the code in Multiplayer, Join a friend, or on the launcher's Play page.";
        let lines = [
            Some((here, color::MUTED)),
            f.notice.clone().map(|n| (n, color::MUTED)),
            Some((hint.to_owned(), color::FAINT)),
        ];
        for (line, tint) in lines.into_iter().flatten() {
            let line = ui.fit(Face::Regular, SMALL, &line, cw);
            ui.text(Face::Regular, SMALL, x, y, tint, &line);
            y += small;
        }
        y + 12.0
    }
    /// The PauseScreen action under a click, if any.
    pub fn click(
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        world: &str,
        quit_label: &str,
        friends: &FriendsView,
        remote: bool,
    ) -> Option<u8> {
        let mut ui = Ui::dry(atlas, batch, (x, y));
        draw(
            &mut ui,
            world,
            quit_label,
            Instant::now() - std::time::Duration::from_secs(5),
            friends,
            remote,
        );
        let hit = ui.hit((x, y))?;
        let nav = id("pause.nav", 0);
        match hit {
            h if h == nav => Some(0),
            h if h == nav + 1 => Some(4),
            h if h == nav + 2 => Some(3),
            h if h == nav + 3 => Some(1),
            h => [0u8, 1, 2, 3, 4, 9, 10, 11, 12]
                .into_iter()
                .find(|&a| act(a) == h),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::Tag;

    #[test]
    fn pause_world_options_and_disconnect_are_clickable_on_servers() {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let friends = FriendsView::default();
        for remote in [false, true] {
            let quit = if remote {
                "Disconnect"
            } else {
                "Save and quit"
            };
            for action in [9u8, 2] {
                let mut batch = HudBatch::new([1280, 720]);
                let (rect, density) = {
                    let mut ui = Ui::dry(&atlas, &mut batch, (0.0, 0.0));
                    pause::draw(
                        &mut ui,
                        "World",
                        quit,
                        Instant::now() - std::time::Duration::from_secs(5),
                        &friends,
                        remote,
                    );
                    let hit = ui
                        .hits
                        .iter()
                        .find(|h| h.id == id("pause", u64::from(action)))
                        .expect("pause action available");
                    (hit.rect, ui.density)
                };
                let k = density / batch.scale;
                let x = (rect[0] + rect[2]) / 2.0 * k;
                let y = (rect[1] + rect[3]) / 2.0 * k;
                assert_eq!(
                    pause::click(&atlas, &mut batch, x, y, "World", quit, &friends, remote),
                    Some(action)
                );
            }
        }
    }

    fn world(saves: &std::path::Path, folder: &str, last_played: i64) {
        crate::save::write_dat(
            &saves.join(folder).join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("LevelName", Tag::String(folder.into())),
                    ("GameType", Tag::Int(0)),
                    ("LastPlayed", Tag::Long(last_played)),
                    ("WorldGenSettings", Tag::compound([("seed", Tag::Long(1))])),
                ]),
            )]),
        )
        .unwrap();
    }
    /// Clicks the middle of button `bid` as the game would; panics if the
    /// page does not show it.
    fn press(title: &mut Title, atlas: &HudAtlas, bid: u64) -> Action {
        let mut batch = HudBatch::new([1280, 720]);
        let (rect, density) = {
            let mut ui = Ui::dry(atlas, &mut batch, (0.0, 0.0));
            title.draw_horizon(&mut ui, 1.0);
            let hit = ui.hits.iter().rev().find(|h| h.id == bid);
            (hit.expect("button on the page").rect, ui.density)
        };
        let k = density / batch.scale;
        let (x, y) = ((rect[0] + rect[2]) / 2.0 * k, (rect[1] + rect[3]) / 2.0 * k);
        title.click_horizon(atlas, &mut batch, x, y)
    }
    fn shows(title: &Title, atlas: &HudAtlas, bid: u64) -> bool {
        let mut batch = HudBatch::new([1280, 720]);
        let mut ui = Ui::dry(atlas, &mut batch, (0.0, 0.0));
        title.draw_horizon(&mut ui, 1.0);
        ui.hits.iter().any(|h| h.id == bid)
    }
    fn selected(title: &Title) -> Option<String> {
        match &title.mode {
            Mode::Select { worlds, selected } => selected.map(|i| worlds[i].folder.clone()),
            _ => panic!("not on the world list"),
        }
    }

    #[test]
    fn horizon_direct_connection_remembers_the_address_after_leaving() {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let saves =
            std::env::temp_dir().join(format!("horizon-front-direct-{}", std::process::id()));
        let mut title = Title::new(saves.clone(), &saves);
        title.last_server = "localhost:25566".into();
        title.mode = Mode::Multiplayer(Screen::list(&saves));
        press(&mut title, &atlas, DIRECT);
        assert_eq!(title.focused_field().unwrap().1, "localhost:25566");
        *title.focused_field().unwrap().1 = "example.org:25567".into();
        press(&mut title, &atlas, BACK);
        press(&mut title, &atlas, DIRECT);
        assert_eq!(title.focused_field().unwrap().1, "example.org:25567");
        *title.focused_field().unwrap().1 = "localhost:25568".into();
        press(&mut title, &atlas, NAV);
        press(&mut title, &atlas, NAV + 2);
        press(&mut title, &atlas, DIRECT);
        assert_eq!(title.focused_field().unwrap().1, "localhost:25568");
    }

    #[test]
    fn horizon_world_delete_confirms_and_keeps_the_selection_in_place() {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let root =
            std::env::temp_dir().join(format!("horizon-front-delete-{}", std::process::id()));
        let saves = root.join("saves");
        for (i, folder) in ["C", "B", "A"].iter().enumerate() {
            world(&saves, folder, 1000 * (i as i64 + 1));
        }
        let mut title = Title::new(saves.clone(), &saves);
        title.mode = Mode::Select {
            worlds: list_worlds(&saves),
            selected: None,
        };
        assert!(!shows(&title, &atlas, DELETE_WORLD));
        press(&mut title, &atlas, WORLD + 1);
        assert_eq!(selected(&title).as_deref(), Some("B"));

        // Cancel leaves the world and its selection.
        press(&mut title, &atlas, DELETE_WORLD);
        assert!(matches!(&title.mode, Mode::Delete { world } if world.folder == "B"));
        press(&mut title, &atlas, BACK);
        assert_eq!(selected(&title).as_deref(), Some("B"));
        assert!(saves.join("B/level.dat").is_file());

        // Deleting selects the world that moved up into the row.
        press(&mut title, &atlas, DELETE_WORLD);
        press(&mut title, &atlas, CONFIRM_DELETE);
        assert!(!saves.join("B").exists());
        assert_eq!(selected(&title).as_deref(), Some("C"));

        // A world open elsewhere is refused on a Horizon page; Back returns
        // to the question.
        let open = crate::save::SessionLock::acquire(&saves.join("C")).unwrap();
        press(&mut title, &atlas, DELETE_WORLD);
        press(&mut title, &atlas, CONFIRM_DELETE);
        assert!(matches!(title.mode, Mode::Error { .. }));
        assert!(!classic(&title.mode));
        press(&mut title, &atlas, BACK);
        assert!(matches!(title.mode, Mode::Delete { .. }));
        drop(open);

        // The last row: the selection moves to the new last world.
        press(&mut title, &atlas, CONFIRM_DELETE);
        assert_eq!(selected(&title).as_deref(), Some("A"));
        press(&mut title, &atlas, DELETE_WORLD);
        press(&mut title, &atlas, CONFIRM_DELETE);
        assert!(
            matches!(&title.mode, Mode::Select { worlds, selected: None } if worlds.is_empty())
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
