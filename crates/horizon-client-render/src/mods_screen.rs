//! The Mods screen, laid out as NeoForge's `ModListScreen`: a searchable, sortable list of the
//! mods on the left and, for the selected one, a scrollable panel with its name, id, version,
//! license, authors, credits and description over Homepage, Issues and Config buttons. There is
//! no switch on it: which mods run is the launcher's choice, as the loader's is in NeoForge.
//! [`ModsScreen::draw`] gives the screen's GUI calls; the options screens host it as
//! `Page::Mods`.
use crate::{
    font::Patch,
    hud::HudAtlas,
    mod_list::ModList,
    pack_screen,
    screen_draw::{self as d, Draw, Measure, TextField},
    text::{Style, Text},
};
use std::{cell::Cell, collections::HashMap, sync::Arc};

/// `HeaderAndFooterLayout(8, 38)`: the room above and below the content.
const HEADER_H: i32 = 8;
const FOOTER_H: i32 = 38;
/// The sidebar's and the info panel's width, and the controls' height.
const SIDEBAR_W: i32 = 150;
const SORT_W: i32 = 50;
const CONTROL_H: i32 = 20;
const PANEL_W: i32 = 250;
/// How far the panel's frame reaches beyond its contents.
const FRAME_PAD: i32 = 2;
/// A list row is its icon and 2 px above and below.
const ICON: i32 = 24;
const ROW_H: i32 = ICON + 4;
/// The tallest the panel's image gets.
const BANNER_H: i32 = 50;
/// The panel's button row.
const BUTTONS_H: i32 = 26;
const MAIN_PAD: i32 = 4;
/// The sidebar and the panel overlap by 3 px (`columnSpacing(-3)`).
const TOTAL_W: i32 = SIDEBAR_W - 3 + PANEL_W;
const SCROLLBAR_W: i32 = 6;
const WHEEL_STEP: f64 = 18.0;
const ROWS: usize = 1000;
const WHITE_TEXT: u32 = 0xFFFFFF;
const RED: u32 = 0xFF5555;
const GOLD: u32 = 0xFFAA00;

/// What a click on the screen asks the game to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The Open Mods Folder button.
    OpenFolder,
    /// The Back button.
    Back,
    /// Config: the settings page of the native mod at this index of `ModList::installed`.
    Config(usize),
    /// Homepage, Issues or the license: this link, through a confirmation.
    Link(String),
    /// A click to copy the mod id or version.
    Copy(String),
}

/// One mod as the screen shows it.
#[derive(Clone, Debug, Default)]
struct Entry {
    id: String,
    name: String,
    version: String,
    authors: String,
    credits: String,
    license: String,
    license_url: Option<String>,
    homepage: Option<String>,
    issues: Option<String>,
    description: String,
    /// The texture its icon is drawn from: a key of [`ModsScreen::icons`].
    icon: Option<String>,
    /// Its place in `ModList::installed`; none for the entries every game has.
    index: Option<usize>,
    /// It has a settings page for Config to open.
    config: bool,
    /// Why it is not running, or that it is not on; stays above the scroller.
    status: Option<Text>,
    /// Dependency-policy notices, independently scrollable below the status.
    note: Option<Text>,
}
impl Entry {
    fn minecraft() -> Self {
        Self {
            id: "minecraft".into(),
            name: "Minecraft".into(),
            version: horizon_core::VANILLA_VERSION.into(),
            authors: "Mojang Studios".into(),
            license: "Minecraft EULA".into(),
            license_url: Some("https://aka.ms/MinecraftEULA".into()),
            homepage: Some("https://minecraft.net".into()),
            issues: Some("https://bugs.mojang.com".into()),
            ..Self::default()
        }
    }
    fn horizon() -> Self {
        Self {
            id: "horizon".into(),
            name: "Horizon".into(),
            version: horizon_core::release::VERSION.into(),
            authors: "Ludovic Marie and the Horizon contributors".into(),
            license: "MIT".into(),
            license_url: Some("https://github.com/horizon-rust/horizon/blob/main/LICENSE".into()),
            homepage: Some("https://playhorizon.dev".into()),
            issues: Some("https://github.com/horizon-rust/horizon/issues".into()),
            description: crate::lang::get("horizon.mods.screen.horizon.description"),
            ..Self::default()
        }
    }
}

/// A scroll position and the scrollbar press that holds it (`AbstractScrollArea`).
#[derive(Clone, Debug, Default)]
struct Scroll {
    amount: f64,
    /// The pointer's last y while the scrollbar is held.
    held: Option<f64>,
}
impl Scroll {
    fn max(content: i32, view: i32) -> i32 {
        (content - view).max(0)
    }
    fn set(&mut self, value: f64, content: i32, view: i32) {
        self.amount = value.clamp(0.0, f64::from(Self::max(content, view)));
    }
    /// The scroller's height.
    fn bar(content: i32, view: i32) -> i32 {
        let bar = ((view * view) as f32 / content.max(1) as f32) as i32;
        bar.clamp(32, (view - 8).max(32)).min(view.max(0))
    }
    /// `AbstractScrollArea.mouseDragged` while the scrollbar is held.
    fn drag(&mut self, y: f64, top: i32, content: i32, view: i32) {
        let Some(last) = self.held else { return };
        if y < f64::from(top) {
            self.set(0.0, content, view);
        } else if y > f64::from(top + view) {
            self.set(f64::from(Self::max(content, view)), content, view);
        } else {
            let max = f64::from(Self::max(content, view).max(1));
            let room = f64::from((view - Self::bar(content, view)).max(1));
            self.set(
                self.amount + (y - last) * (max / room).max(1.0),
                content,
                view,
            );
        }
        self.held = Some(y);
    }
    /// The scrollbar's sprites, when there is anything to scroll.
    fn draw(&self, out: &mut Vec<Draw>, x: i32, top: i32, content: i32, view: i32) {
        let max = Self::max(content, view);
        if max == 0 {
            return;
        }
        let bar = Self::bar(content, view);
        let y = (self.amount as i32 * (view - bar) / max + top).max(top);
        d::sprite(out, "widget/scroller_background", x, top, SCROLLBAR_W, view);
        d::sprite(out, "widget/scroller", x, y, SCROLLBAR_W, bar);
    }
}

/// The rectangles of the screen, `[x, y, width, height]`.
struct Geo {
    search: [i32; 4],
    sort: [i32; 4],
    list: [i32; 4],
    /// The scrolling part of the info panel.
    panel: [i32; 4],
    /// Homepage, Issues and Config.
    buttons: [[i32; 4]; 3],
    /// Open Mods Folder and Back.
    footer: [[i32; 4]; 2],
}

fn hit(r: [i32; 4], x: f64, y: f64) -> bool {
    x >= f64::from(r[0])
        && y >= f64::from(r[1])
        && x < f64::from(r[0] + r[2])
        && y < f64::from(r[1] + r[3])
}

/// A piece of the panel that copies something when clicked.
struct Piece {
    text: Text,
    x: i32,
    w: i32,
    copy: String,
    hover: &'static str,
}
enum Kind {
    Image {
        texture: String,
        size: i32,
    },
    Lines {
        lines: Vec<(Text, i32)>,
        centered: bool,
    },
    Notice {
        lines: Vec<(Text, i32)>,
    },
    Subtitle(Vec<Piece>),
    License {
        text: Text,
        width: i32,
        link: Option<String>,
    },
    Rule,
}
struct Placed {
    /// Below the top of the panel's contents.
    y: i32,
    kind: Kind,
}

fn tr(key: &str) -> Text {
    Text::translate(key, &[])
}
/// `Authors: x` and its kin: the label gray and bold, the value white and plain.
fn field(key: &str, value: &str, underlined: bool) -> Text {
    let value = Text::styled(
        value,
        Style {
            color: Some(WHITE_TEXT),
            bold: Some(false),
            underlined: underlined.then_some(true),
            ..Style::default()
        },
    );
    Text::translate(key, &[value]).with_style(&Style {
        color: Some(d::GRAY),
        bold: Some(true),
        ..Style::default()
    })
}

/// The Mods screen and its state.
#[derive(Clone, Debug)]
pub struct ModsScreen {
    pub width: i32,
    pub height: i32,
    in_world: bool,
    entries: Vec<Entry>,
    /// The mods' icons by [`Entry::icon`], 128x128 RGBA, for [`Self::bind_icons`].
    pub icons: HashMap<String, Arc<[u8]>>,
    search: TextField,
    descending: bool,
    /// Indices into `entries` that the search keeps, in the order shown.
    shown: Vec<usize>,
    selected: Option<usize>,
    list: Scroll,
    list_focused: bool,
    panel: Scroll,
    /// The panel's contents' height as last drawn, for scrolling.
    panel_content: Cell<i32>,
    panel_pinned: Cell<i32>,
}

impl ModsScreen {
    /// `ModListScreen.init`: Minecraft, Horizon, then the installed mods.
    pub fn new(width: i32, height: i32, in_world: bool, mods: &ModList) -> Self {
        let mut icons = HashMap::new();
        let mut entries = vec![Entry::minecraft(), Entry::horizon()];
        for (index, installed) in mods.installed.iter().enumerate() {
            let info = mods.info.get(&installed.id);
            let key = format!("mod/{index}/icon");
            let png = info
                .and_then(|i| i.icon.as_deref())
                .and_then(pack_screen::load_icon);
            let has_icon = png.is_some();
            if let Some(png) = png {
                icons.insert(key.clone(), png);
            }
            let status = if let Some(problem) = &installed.problem {
                Some(Text::literal(problem.clone()).with_color(RED))
            } else if !mods.is_on(index) {
                Some(tr("horizon.mods.screen.off").with_color(d::GRAY))
            } else {
                None
            };
            let mut notes = Vec::new();
            // Status does not hide load notices. Every applied override and
            // discouraged pair remains readable in this row's scrollable panel.
            if let Some(info) = info {
                for (key, args) in &info.warnings {
                    let args: Vec<_> = args.iter().map(|arg| Text::literal(arg.clone())).collect();
                    notes.push(Text::translate(key, &args).with_color(GOLD));
                }
            }
            let note = notes
                .into_iter()
                .reduce(|left, right| left.append(Text::literal("\n\n")).append(right));
            entries.push(Entry {
                id: installed.id.clone(),
                name: info.map_or_else(|| installed.title.clone(), |i| i.name.clone()),
                version: info.map(|i| i.version.clone()).unwrap_or_default(),
                authors: info.map(|i| i.authors.join(", ")).unwrap_or_default(),
                credits: info.map(|i| i.credits.clone()).unwrap_or_default(),
                license: info.map(|i| i.license.clone()).unwrap_or_default(),
                license_url: info.and_then(|i| i.license_url.clone()),
                homepage: info.and_then(|i| i.homepage.clone()),
                issues: info.and_then(|i| i.issues.clone()),
                description: installed.description.clone(),
                icon: has_icon.then_some(key),
                index: Some(index),
                config: installed
                    .native
                    .as_ref()
                    .is_some_and(|page| !page.settings.is_empty()),
                status,
                note,
            });
        }
        let mut search = TextField::new(0, 0, SIDEBAR_W - SORT_W - 4, CONTROL_H);
        search.max_length = 64;
        search.hint = Some(tr("horizon.mods.screen.search").with_color(d::GRAY));
        let mut screen = Self {
            width,
            height,
            in_world,
            entries,
            icons,
            search,
            descending: false,
            shown: Vec::new(),
            selected: None,
            list: Scroll::default(),
            list_focused: false,
            panel: Scroll::default(),
            panel_content: Cell::new(0),
            panel_pinned: Cell::new(0),
        };
        screen.place();
        screen.refresh();
        screen
    }

    fn geo(&self) -> Geo {
        let content = (self.height - HEADER_H - FOOTER_H).max(0);
        let left = ((self.width - TOTAL_W) / 2).max(0);
        let top = HEADER_H - FRAME_PAD;
        let panel_x = left + SIDEBAR_W - 3;
        let view = (content - BUTTONS_H).max(0);
        let button_w = PANEL_W / 3 - 1;
        let gap = (PANEL_W + 2 * FRAME_PAD - 3 * button_w) / 4;
        let button_y = HEADER_H + view + (BUTTONS_H - CONTROL_H);
        let footer_w = ((self.width - 4) / 2).clamp(1, 150);
        let footer_x = (self.width - (2 * footer_w + 4)) / 2;
        let footer_y = self.height - FOOTER_H + 6 + 5;
        Geo {
            search: [left, top, SIDEBAR_W - SORT_W - 4, CONTROL_H],
            sort: [left + SIDEBAR_W - SORT_W, top, SORT_W, CONTROL_H],
            list: [
                left,
                top + CONTROL_H + 6,
                SIDEBAR_W,
                (content - CONTROL_H - 5).max(0),
            ],
            panel: [panel_x, HEADER_H, PANEL_W, view],
            buttons: [0, 1, 2].map(|i| {
                [
                    panel_x - FRAME_PAD + gap + i * (button_w + gap),
                    button_y,
                    button_w,
                    CONTROL_H,
                ]
            }),
            footer: [0, 1].map(|i| [footer_x + i * (footer_w + 4), footer_y, footer_w, 20]),
        }
    }
    /// Puts the search box where the layout has it.
    fn place(&mut self) {
        let [x, y, w, h] = self.geo().search;
        (
            self.search.x,
            self.search.y,
            self.search.width,
            self.search.height,
        ) = (x, y, w, h);
    }
    pub fn resize(&mut self, width: i32, height: i32) {
        (self.width, self.height) = (width, height);
        self.place();
        self.clamp_scrolls();
    }
    fn list_content(&self) -> i32 {
        self.shown.len() as i32 * ROW_H + 4
    }
    fn clamp_scrolls(&mut self) {
        let g = self.geo();
        let (list, panel) = (self.list_content(), self.panel_content.get());
        self.list.set(self.list.amount, list, g.list[3]);
        self.panel.set(self.panel.amount, panel, g.panel[3]);
    }
    /// `updateModsList`: the search's matches, in the sort order; the selection stays when it can,
    /// and a lone match is selected.
    fn refresh(&mut self) {
        let query = self.search.value.to_lowercase();
        let mut shown: Vec<usize> = (0..self.entries.len())
            .filter(|&i| query.is_empty() || self.entries[i].name.to_lowercase().contains(&query))
            .collect();
        shown.sort_by_key(|&i| self.entries[i].name.to_lowercase());
        if self.descending {
            shown.reverse();
        }
        let before = self.selected;
        if !before.is_some_and(|s| shown.contains(&s)) {
            self.selected = (shown.len() == 1).then(|| shown[0]);
        }
        if self.selected != before {
            self.panel.amount = 0.0;
        }
        self.shown = shown;
        self.clamp_scrolls();
    }
    fn select(&mut self, entry: usize) {
        if self.selected != Some(entry) {
            self.panel.amount = 0.0;
        }
        self.selected = Some(entry);
    }
    fn current(&self) -> Option<&Entry> {
        self.entries.get(self.selected?)
    }
    /// The up and down keys: the previous or next mod in the list.
    pub fn step(&mut self, delta: i32) {
        let Some(last) = self.shown.len().checked_sub(1) else {
            return;
        };
        let at = self
            .selected
            .and_then(|s| self.shown.iter().position(|&i| i == s));
        let next = match at {
            Some(at) => (at as i32 + delta).clamp(0, last as i32) as usize,
            None if delta > 0 => 0,
            None => last,
        };
        self.select(self.shown[next]);
        let view = self.geo().list[3];
        let content = self.list_content();
        let (top, bottom) = (next as i32 * ROW_H, (next as i32 + 1) * ROW_H + 4);
        let amount = self.list.amount;
        if f64::from(top) < amount {
            self.list.set(f64::from(top), content, view);
        } else if f64::from(bottom) > amount + f64::from(view) {
            self.list.set(f64::from(bottom - view), content, view);
        }
    }

    pub fn search_focused(&self) -> bool {
        self.search.focused
    }
    /// Typed text, when the search box has the focus.
    pub fn type_text(&mut self, m: &Measure, s: &str) {
        if self.search.focused && self.search.insert_text(m, s) {
            self.refresh();
        }
    }
    /// Backspace in the search box.
    pub fn erase(&mut self, m: &Measure, count: isize) {
        if self.search.focused && self.search.delete_chars(m, -count) {
            self.refresh();
        }
    }
    /// Ctrl+A/C/V/X in the search box; false when it does not have the focus.
    pub fn shortcut(
        &mut self,
        m: &Measure,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if !self.search.focused {
            return false;
        }
        if self.search.shortcut(m, shortcut, clipboard) {
            self.refresh();
        }
        true
    }

    /// What the panel holds for `e`, each item's place below the panel's top, and the height.
    fn items(&self, m: &Measure, e: &Entry, width: i32) -> (Vec<Placed>, i32, i32) {
        let lines = |text: Text| d::label_lines(m, &text, width, ROWS, &Style::default());
        let mut out = Vec::new();
        let mut y = FRAME_PAD;
        // The banner, else the logo: a mod has its icon only.
        if let Some(texture) = &e.icon {
            out.push(Placed {
                y,
                kind: Kind::Image {
                    texture: texture.clone(),
                    size: BANNER_H,
                },
            });
            y += BANNER_H;
        }
        y += MAIN_PAD;
        let name = lines(Text::literal(e.name.clone()));
        let rows = name.len() as i32;
        out.push(Placed {
            y,
            kind: Kind::Lines {
                lines: name,
                centered: true,
            },
        });
        y += 9 * rows;
        // `<id> <version>`, each copied by a click.
        let space = m.width_str(" ");
        let mut pieces = Vec::new();
        let mut at = 0;
        if !e.id.is_empty() {
            let text = Text::styled(
                e.id.clone(),
                Style {
                    color: Some(d::GRAY),
                    italic: Some(true),
                    ..Style::default()
                },
            );
            let w = m.width(&text);
            pieces.push(Piece {
                text,
                x: at,
                w,
                copy: e.id.clone(),
                hover: "horizon.mods.screen.copy_id",
            });
            at += w;
        }
        if !e.version.is_empty() {
            if !pieces.is_empty() {
                at += space;
            }
            let text = Text::colored(e.version.clone(), d::GRAY);
            let w = m.width(&text);
            pieces.push(Piece {
                text,
                x: at,
                w,
                copy: e.version.clone(),
                hover: "horizon.mods.screen.copy_version",
            });
            at += w;
        }
        let start = (width - at) / 2;
        for piece in &mut pieces {
            piece.x += start;
        }
        out.push(Placed {
            y,
            kind: Kind::Subtitle(pieces),
        });
        // Room for an update notice, and its spacer.
        y += 9 + 10;
        if let Some(status) = &e.status {
            let status = lines(status.clone());
            let rows = status.len() as i32;
            out.push(Placed {
                y,
                kind: Kind::Lines {
                    lines: status,
                    centered: true,
                },
            });
            y += 9 * rows + 9;
        }
        let pinned = y;
        if let Some(note) = &e.note {
            let note = d::label_lines(m, note, width - 12, usize::MAX, &Style::default());
            let rows = note.len() as i32;
            out.push(Placed {
                y,
                kind: Kind::Notice { lines: note },
            });
            y += 9 * rows + 9;
        }
        let license = if e.license.is_empty() {
            crate::lang::get("horizon.mods.screen.license.none")
        } else {
            e.license.clone()
        };
        let text = field(
            "horizon.mods.screen.license",
            &license,
            e.license_url.is_some(),
        );
        let text = if m.width(&text) > width {
            d::clip(m, &text, width)
        } else {
            text
        };
        let text_width = m.width(&text).min(width);
        out.push(Placed {
            y,
            kind: Kind::License {
                text,
                width: text_width,
                link: e.license_url.clone(),
            },
        });
        y += 9;
        for (key, value) in [
            ("horizon.mods.screen.authors", &e.authors),
            ("horizon.mods.screen.credits", &e.credits),
        ] {
            if value.is_empty() {
                continue;
            }
            let block = lines(field(key, value, false));
            let rows = block.len() as i32;
            out.push(Placed {
                y,
                kind: Kind::Lines {
                    lines: block,
                    centered: false,
                },
            });
            y += 9 * rows;
        }
        if !e.description.is_empty() {
            y += MAIN_PAD;
            out.push(Placed {
                y,
                kind: Kind::Rule,
            });
            y += 1 + MAIN_PAD;
            let block = lines(Text::literal(e.description.clone()));
            let rows = block.len() as i32;
            out.push(Placed {
                y,
                kind: Kind::Lines {
                    lines: block,
                    centered: false,
                },
            });
            y += 9 * rows;
        }
        (out, y + FRAME_PAD, pinned)
    }
    /// The panel's items for the selected mod, narrower by the scrollbar when they need one.
    fn panel_items(&self, m: &Measure, e: &Entry) -> (Vec<Placed>, i32, i32, i32) {
        let view = self.geo().panel[3];
        let (items, content, pinned) = self.items(m, e, PANEL_W);
        if content <= view {
            return (items, content, PANEL_W, pinned);
        }
        let width = PANEL_W - SCROLLBAR_W;
        let (items, content, pinned) = self.items(m, e, width);
        // Keep at least three body lines and a third of the view useful.
        // Otherwise the whole pane scrolls, including an unusually tall status.
        let minimum_body = 27.max(view / 3);
        let pinned = if pinned + minimum_body <= view {
            pinned
        } else {
            0
        };
        (items, content, width, pinned)
    }

    /// A left click: what it asks of the game, and whether it sounds as a button.
    pub fn click(&mut self, m: &Measure, x: f64, y: f64) -> (Action, bool) {
        self.place();
        let g = self.geo();
        for (i, r) in g.footer.into_iter().enumerate() {
            if hit(r, x, y) {
                self.search.focused = false;
                let action = if i == 0 {
                    Action::OpenFolder
                } else {
                    Action::Back
                };
                return (action, true);
            }
        }
        if hit(g.search, x, y) {
            self.search.focused = true;
            self.list_focused = false;
            self.search.click(m, x, false, false);
            return (Action::None, false);
        }
        self.search.focused = false;
        if hit(g.sort, x, y) {
            self.descending = !self.descending;
            self.refresh();
            return (Action::None, true);
        }
        let [lx, ly, lw, lh] = g.list;
        if hit(g.list, x, y) {
            self.list_focused = true;
            let content = self.list_content();
            let scrollable = Scroll::max(content, lh) > 0;
            if scrollable && x >= f64::from(lx + lw - SCROLLBAR_W) {
                self.list.held = Some(y);
                return (Action::None, false);
            }
            let row_w = lw - 6 - if scrollable { SCROLLBAR_W } else { 0 };
            if (f64::from(lx + 3)..f64::from(lx + 3 + row_w)).contains(&x) {
                let below = y - f64::from(ly + 2) + self.list.amount;
                if below >= 0.0
                    && let Some(&entry) = self.shown.get((below / f64::from(ROW_H)) as usize)
                {
                    self.select(entry);
                }
            }
            return (Action::None, false);
        }
        let Some(e) = self.current() else {
            return (Action::None, false);
        };
        for (i, r) in g.buttons.into_iter().enumerate() {
            if !hit(r, x, y) {
                continue;
            }
            let link = |url: &Option<String>| url.clone().map_or(Action::None, Action::Link);
            let action = match i {
                0 => link(&e.homepage),
                1 => link(&e.issues),
                _ => match (e.config, e.index) {
                    (true, Some(index)) => Action::Config(index),
                    _ => Action::None,
                },
            };
            return (action.clone(), action != Action::None);
        }
        if !hit(g.panel, x, y) {
            return (Action::None, false);
        }
        let e = e.clone();
        let (items, content, _, pinned) = self.panel_items(m, &e);
        self.panel_content.set(content);
        self.panel_pinned.set(pinned);
        let view = g.panel[3];
        if Scroll::max(content, view) > 0
            && y >= f64::from(g.panel[1] + pinned)
            && x >= f64::from(g.panel[0] + PANEL_W + FRAME_PAD - SCROLLBAR_W)
        {
            self.panel.held = Some(y);
            return (Action::None, false);
        }
        for placed in &items {
            let fixed = placed.y < pinned;
            if !fixed && y < f64::from(g.panel[1] + pinned) {
                continue;
            }
            let top = g.panel[1] - if fixed { 0 } else { self.panel.amount as i32 };
            let row = |left: i32, w: i32| [g.panel[0] + left, top + placed.y, w, 9];
            match &placed.kind {
                Kind::Subtitle(pieces) => {
                    for piece in pieces {
                        if hit(row(piece.x, piece.w), x, y) {
                            return (Action::Copy(piece.copy.clone()), false);
                        }
                    }
                }
                Kind::License {
                    width,
                    link: Some(url),
                    ..
                } if hit(row(0, *width), x, y) => {
                    return (Action::Link(url.clone()), false);
                }
                _ => {}
            }
        }
        (Action::None, false)
    }
    /// The pointer moved with the button held: a held scrollbar follows it.
    pub fn drag(&mut self, y: f64) {
        let g = self.geo();
        let content = self.list_content();
        self.list.drag(y, g.list[1], content, g.list[3]);
        let pinned = self.panel_pinned.get();
        self.panel.drag(
            y,
            g.panel[1] + pinned,
            self.panel_content.get() - pinned,
            g.panel[3] - pinned,
        );
    }
    pub fn release(&mut self) {
        self.list.held = None;
        self.panel.held = None;
    }
    /// The mouse wheel over the list or the panel.
    pub fn scroll(&mut self, x: f64, y: f64, amount: f64) {
        let g = self.geo();
        if hit(g.list, x, y) {
            let content = self.list_content();
            self.list
                .set(self.list.amount - amount * WHEEL_STEP, content, g.list[3]);
        } else if hit(
            [
                g.panel[0] - FRAME_PAD,
                g.panel[1],
                PANEL_W + 2 * FRAME_PAD,
                g.panel[3],
            ],
            x,
            y,
        ) {
            let content = self.panel_content.get();
            self.panel
                .set(self.panel.amount - amount * WHEEL_STEP, content, g.panel[3]);
        }
    }

    /// Points the icon blits of `draws` at the atlas's icon slots and writes the icons into them.
    pub fn bind_icons(&self, draws: &mut [Draw], atlas: &HudAtlas, patches: &mut Vec<Patch>) {
        pack_screen::bind_icon_slots(&self.icons, draws, atlas, patches);
    }

    /// The screen's calls over the menu background. `blink_ms` is the time since the search box
    /// took the focus.
    pub fn draw(&self, m: &Measure, mouse: (i32, i32), blink_ms: u64, now_ms: i64) -> Vec<Draw> {
        let g = self.geo();
        let mut out = Vec::new();
        let prefix = if self.in_world { "inworld_" } else { "" };
        let separator_at = |out: &mut Vec<Draw>, name: &str, x: i32, y: i32, w: i32| {
            d::blit(
                out,
                &format!("textures/gui/{prefix}{name}_separator.png"),
                x,
                y,
                (0.0, 0.0),
                w,
                2,
                [32, 2],
            );
        };
        separator_at(&mut out, "header", 0, HEADER_H - 2, self.width);
        separator_at(&mut out, "footer", 0, self.height - FOOTER_H, self.width);
        let mut tooltip = None;

        // The sidebar: the search box and the sort button over the list.
        self.search.draw(&mut out, m, blink_ms, None);
        let [sx, sy, sw, sh] = g.sort;
        let sort_hover = d::inside(mouse, sx, sy, sw, sh);
        crate::widgets::widget_cursor(sort_hover, true);
        let sort = if self.descending { "z_to_a" } else { "a_to_z" };
        let name = format!("horizon.mods.screen.sort.{sort}");
        d::button(
            &mut out,
            m,
            &tr(&name),
            sx,
            sy,
            sw,
            sh,
            true,
            sort_hover,
            now_ms,
        );
        if sort_hover {
            tooltip = Some(tr(&format!("{name}.hover")));
        }
        // The info panel's frame, under the list so the list's scrollbar stays on top where
        // the two overlap.
        let [px, py, _, view] = g.panel;
        // `BackgroundWithPipingWidget`: the list's background and separators, with the frame's
        // 2 px inset on every side for the separators and the scrolling contents between them.
        let frame_w = PANEL_W + 2 * FRAME_PAD;
        d::blit(
            &mut out,
            &format!("textures/gui/{prefix}menu_list_background.png"),
            px - FRAME_PAD,
            py - FRAME_PAD,
            (0.0, 0.0),
            frame_w,
            view + 2 * FRAME_PAD,
            [32, 32],
        );
        separator_at(&mut out, "header", px - FRAME_PAD, py - FRAME_PAD, frame_w);
        separator_at(&mut out, "footer", px - FRAME_PAD, py + view, frame_w);

        self.draw_list(&mut out, m, &g);

        // The info panel's contents.
        let selected = self.current();
        if let Some(e) = selected {
            let (items, content, width, pinned) = self.panel_items(m, e);
            self.panel_content.set(content);
            self.panel_pinned.set(pinned);
            let scroll = self.panel.amount.min(f64::from(Scroll::max(content, view))) as i32;
            out.push(Draw::Scissor {
                x0: px - FRAME_PAD,
                y0: py,
                x1: px + PANEL_W + FRAME_PAD,
                y1: py + pinned.min(view),
            });
            let mut body = false;
            for placed in &items {
                if !body && placed.y >= pinned {
                    out.push(Draw::NoScissor);
                    out.push(Draw::Scissor {
                        x0: px - FRAME_PAD,
                        y0: py + pinned.min(view),
                        x1: px + PANEL_W + FRAME_PAD,
                        y1: py + view,
                    });
                    body = true;
                }
                let y = py + placed.y - if body { scroll } else { 0 };
                match &placed.kind {
                    Kind::Image { texture, size } => {
                        let x = px + (PANEL_W - size) / 2;
                        d::blit(
                            &mut out,
                            texture,
                            x,
                            y,
                            (0.0, 0.0),
                            *size,
                            *size,
                            [*size, *size],
                        );
                    }
                    Kind::Lines { lines, centered } => {
                        for (i, (line, w)) in lines.iter().enumerate() {
                            let x = if *centered { px + (width - w) / 2 } else { px };
                            d::text(&mut out, line.clone(), x, y + 9 * i as i32, d::WHITE, true);
                        }
                    }
                    Kind::Notice { lines } => {
                        for (i, (line, _)) in lines.iter().enumerate() {
                            d::text(
                                &mut out,
                                line.clone(),
                                px + 6,
                                y + 9 * i as i32,
                                d::WHITE,
                                true,
                            );
                        }
                    }
                    Kind::Subtitle(pieces) => {
                        for piece in pieces {
                            let x = px + piece.x;
                            d::text(&mut out, piece.text.clone(), x, y, d::WHITE, true);
                            if d::inside(mouse, x, y, piece.w, 9)
                                && hit(g.panel, mouse.0.into(), mouse.1.into())
                            {
                                tooltip = Some(tr(piece.hover));
                            }
                        }
                    }
                    Kind::License { text, .. } => {
                        d::text(&mut out, text.clone(), px, y, d::WHITE, true);
                    }
                    Kind::Rule => {
                        d::fill(
                            &mut out,
                            px + 4,
                            y + 1,
                            px + width - 4,
                            y + 2,
                            0xFF00_0000u32 as i32,
                        );
                        d::fill(
                            &mut out,
                            px + 4,
                            y,
                            px + width - 4,
                            y + 1,
                            0xFFAA_AAAAu32 as i32,
                        );
                    }
                }
            }
            out.push(Draw::NoScissor);
            self.panel.draw(
                &mut out,
                px + PANEL_W + FRAME_PAD - SCROLLBAR_W,
                py + pinned,
                content - pinned,
                view - pinned,
            );
        }
        let e = selected;
        let enabled = [
            e.is_some_and(|e| e.homepage.is_some()),
            e.is_some_and(|e| e.issues.is_some()),
            e.is_some_and(|e| e.config && e.index.is_some()),
        ];
        let labels = ["homepage", "issues", "config"];
        for ((r, active), label) in g.buttons.into_iter().zip(enabled).zip(labels) {
            let hover = d::inside(mouse, r[0], r[1], r[2], r[3]);
            crate::widgets::widget_cursor(hover, active);
            let text = tr(&format!("horizon.mods.screen.{label}"));
            d::button(
                &mut out,
                m,
                &text,
                r[0],
                r[1],
                r[2],
                r[3],
                active,
                hover && active,
                now_ms,
            );
        }

        // The footer.
        let labels = [tr("horizon.mods.open_folder"), tr("gui.back")];
        for (i, (r, label)) in g.footer.into_iter().zip(&labels).enumerate() {
            let hover = d::inside(mouse, r[0], r[1], r[2], r[3]);
            crate::widgets::widget_cursor(hover, true);
            if i == 1 {
                crate::widgets::declare_action(
                    crate::widgets::Action::Back,
                    [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32],
                    true,
                );
            }
            d::button(
                &mut out, m, label, r[0], r[1], r[2], r[3], true, hover, now_ms,
            );
        }
        if let Some(tooltip) = tooltip {
            d::tooltip(&mut out, m, &tooltip, mouse);
        }
        out
    }

    /// The sidebar's `ModsList`: an `ObjectSelectionList` of 28 px rows.
    fn draw_list(&self, out: &mut Vec<Draw>, m: &Measure, g: &Geo) {
        let [x, y, w, h] = g.list;
        let prefix = if self.in_world { "inworld_" } else { "" };
        let amount = self.list.amount as i32;
        d::blit(
            out,
            &format!("textures/gui/{prefix}menu_list_background.png"),
            x,
            y,
            ((x + w) as f32, (y + h + amount) as f32),
            w,
            h,
            [32, 32],
        );
        out.push(Draw::Scissor {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        });
        let content = self.list_content();
        let scrollable = Scroll::max(content, h) > 0;
        let row_w = w - 6 - if scrollable { SCROLLBAR_W } else { 0 };
        let left = x + 3;
        for (position, &i) in self.shown.iter().enumerate() {
            let top = y + 2 - amount + position as i32 * ROW_H;
            if top + ROW_H < y || top > y + h {
                continue;
            }
            let e = &self.entries[i];
            if self.selected == Some(i) {
                let border = if self.list_focused { -1 } else { -8355712 };
                d::fill(out, left, top, left + row_w, top + ROW_H, border);
                d::fill(
                    out,
                    left + 1,
                    top + 1,
                    left + row_w - 1,
                    top + ROW_H - 1,
                    -16777216,
                );
            }
            let (cx, cy) = (left + 2, top + 2);
            let mut text_left = cx + 2;
            if let Some(icon) = &e.icon {
                d::blit(out, icon, cx, cy, (0.0, 0.0), ICON, ICON, [ICON, ICON]);
                text_left += ICON + 4;
            }
            let room = row_w - (text_left - cx) - 4;
            if room <= 0 {
                continue;
            }
            d::string_widget(
                out,
                m,
                &Text::literal(e.name.clone()),
                text_left,
                cy + 4,
                room,
            );
            if !e.version.is_empty() {
                let version = Text::colored(e.version.clone(), d::GRAY);
                d::string_widget(out, m, &version, text_left, cy + 4 + 9, room);
            }
        }
        out.push(Draw::NoScissor);
        d::blit(
            out,
            &format!("textures/gui/{prefix}header_separator.png"),
            x,
            y - 2,
            (0.0, 0.0),
            w,
            2,
            [32, 2],
        );
        d::blit(
            out,
            &format!("textures/gui/{prefix}footer_separator.png"),
            x,
            y + h,
            (0.0, 0.0),
            w,
            2,
            [32, 2],
        );
        self.list.draw(out, x + w - SCROLLBAR_W, y, content, h);
    }
}

/// The atlas sprite of the main menu's Mods button, as the icons beside it are named.
pub const ICON_SPRITE: &str = "gui/sprites/icon/mods";
/// A cube, drawn in three greys for its top and two sides: `#` light, `o` middle, `x` dark.
const ICON_ART: [&str; 15] = [
    ".......#.......",
    ".....#####.....",
    "...#########...",
    ".#############.",
    "...#########...",
    ".....#####.....",
    ".......#.......",
    ".oooooo#xxxxxx.",
    ".oooooo#xxxxxx.",
    ".oooooo#xxxxxx.",
    ".oooooo#xxxxxx.",
    "..ooooo#xxxxx..",
    "...oooo#xxxx...",
    ".....oo#xx.....",
    ".......#.......",
];

/// The Mods button's icon for the atlas: `(sprite, width, height, RGBA texels)`.
pub fn icon_image() -> (String, u32, u32, Vec<u8>) {
    let pixels = ICON_ART
        .iter()
        .flat_map(|row| row.chars())
        .flat_map(|c| match c {
            '#' => [255, 255, 255, 255],
            'o' => [176, 176, 176, 255],
            'x' => [112, 112, 112, 255],
            _ => [0, 0, 0, 0],
        })
        .collect();
    (ICON_SPRITE.to_owned(), 15, 15, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_list::{Installed, ModInfo, NativePage};

    #[test]
    fn the_mods_icon_is_a_fifteen_pixel_square_with_a_clear_corner() {
        assert!(ICON_ART.iter().all(|row| row.chars().count() == 15));
        let (name, w, h, pixels) = icon_image();
        assert_eq!((name.as_str(), w, h), (ICON_SPRITE, 15, 15));
        assert_eq!(pixels.len(), 15 * 15 * 4);
        assert_eq!(pixels[3], 0, "the corner is clear");
        assert_eq!(pixels[(7 * 4) + 3], 255, "the cube's top point is drawn");
    }

    fn atlas() -> HudAtlas {
        HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/horizon-test-assets"),
        ))
        .unwrap()
    }
    fn installed(id: &str, name: &str, native: Option<NativePage>) -> (Installed, ModInfo) {
        (
            Installed {
                id: id.into(),
                title: format!("{name} 1.0"),
                description: format!("{name} does things."),
                problem: None,
                dev: false,
                native,
            },
            ModInfo {
                name: name.into(),
                version: "1.0".into(),
                authors: vec!["Alex".into(), "Sam".into()],
                homepage: Some("https://example.org/zoom".into()),
                ..ModInfo::default()
            },
        )
    }
    fn mods(list: &[(Installed, ModInfo)]) -> ModList {
        let mut mods = ModList::default();
        for (row, info) in list {
            mods.installed.push(row.clone());
            mods.info.insert(row.id.clone(), info.clone());
        }
        mods
    }
    fn names(screen: &ModsScreen) -> Vec<&str> {
        screen
            .shown
            .iter()
            .map(|&i| screen.entries[i].name.as_str())
            .collect()
    }
    fn centre(r: [i32; 4]) -> (f64, f64) {
        (f64::from(r[0] + r[2] / 2), f64::from(r[1] + r[3] / 2))
    }

    #[test]
    fn the_list_has_the_built_in_entries_and_follows_the_search_and_the_sort() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let mut screen =
            ModsScreen::new(427, 240, false, &mods(&[installed("zoom", "Zoom", None)]));
        assert_eq!(names(&screen), ["Horizon", "Minecraft", "Zoom"]);
        // The sort button reverses the order.
        let (action, sound) =
            screen.click(&m, centre(screen.geo().sort).0, centre(screen.geo().sort).1);
        assert_eq!((action, sound), (Action::None, true));
        assert_eq!(names(&screen), ["Zoom", "Minecraft", "Horizon"]);
        // The search keeps the names that contain it, in any case.
        let (x, y) = centre(screen.geo().search);
        screen.click(&m, x, y);
        assert!(screen.search_focused());
        screen.type_text(&m, "OOM");
        assert_eq!(names(&screen), ["Zoom"]);
        // A lone match is selected; a click elsewhere takes the focus off the box.
        assert_eq!(screen.current().map(|e| e.id.as_str()), Some("zoom"));
        let (x, y) = centre(screen.geo().panel);
        screen.click(&m, x, y);
        assert!(!screen.search_focused());
        screen.search.focused = true;
        for _ in 0..3 {
            screen.erase(&m, 1);
        }
        // Everything is listed again and the selection stays.
        assert_eq!(names(&screen).len(), 3);
        assert_eq!(screen.current().map(|e| e.id.as_str()), Some("zoom"));
        // A search that drops the selected mod leaves nothing selected.
        screen.type_text(&m, "x");
        assert!(names(&screen).is_empty());
        assert!(screen.current().is_none());
    }

    #[test]
    fn the_panel_buttons_follow_the_selected_mod() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let page = NativePage {
            settings: vec![crate::mod_list::NativeSetting {
                key: "open".into(),
                label: "Open".into(),
                tooltip: None,
                value: crate::mod_list::NativeSettingValue::Toggle(true),
            }],
            ..NativePage::default()
        };
        let list = mods(&[
            installed("zoom", "Zoom", Some(page)),
            installed("plain", "Plain", None),
        ]);
        let mut screen = ModsScreen::new(427, 240, false, &list);
        let click_row = |screen: &mut ModsScreen, name: &str| {
            let g = screen.geo();
            let at = screen
                .shown
                .iter()
                .position(|&i| screen.entries[i].name == name)
                .unwrap();
            let y = g.list[1] + 2 + at as i32 * ROW_H + ROW_H / 2;
            screen.click(&m, f64::from(g.list[0] + 20), f64::from(y))
        };
        let press = |screen: &mut ModsScreen, button: usize| {
            let (x, y) = centre(screen.geo().buttons[button]);
            screen.click(&m, x, y)
        };
        // Nothing selected: no button does anything.
        assert_eq!(press(&mut screen, 0), (Action::None, false));
        assert_eq!(click_row(&mut screen, "Zoom"), (Action::None, false));
        assert_eq!(
            press(&mut screen, 0),
            (Action::Link("https://example.org/zoom".into()), true)
        );
        // No issues link; Config opens the native mod's page, by its place in the installed list.
        assert_eq!(press(&mut screen, 1), (Action::None, false));
        assert_eq!(press(&mut screen, 2), (Action::Config(0), true));
        click_row(&mut screen, "Plain");
        assert_eq!(press(&mut screen, 2), (Action::None, false));
        // Minecraft's links are its own.
        click_row(&mut screen, "Minecraft");
        assert_eq!(
            press(&mut screen, 1),
            (Action::Link("https://bugs.mojang.com".into()), true)
        );
    }

    #[test]
    fn the_footer_asks_for_the_folder_and_the_way_back() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let mut screen = ModsScreen::new(427, 240, false, &ModList::default());
        let actions: Vec<_> = screen
            .geo()
            .footer
            .into_iter()
            .map(|r| {
                let (x, y) = centre(r);
                screen.click(&m, x, y)
            })
            .collect();
        assert_eq!(actions, [(Action::OpenFolder, true), (Action::Back, true)]);
    }

    /// Literal rectangles, not `geo()` read back: two 150 px buttons 4 px apart, centred, 11 px
    /// below the footer separator's row (`height - 38`); narrow screens shrink both equally.
    #[test]
    fn the_footer_buttons_sit_at_fixed_rectangles() {
        let footer = |w, h| {
            ModsScreen::new(w, h, false, &ModList::default())
                .geo()
                .footer
        };
        assert_eq!(footer(427, 240), [[61, 213, 150, 20], [215, 213, 150, 20]]);
        // Too narrow for 150 px: (240 - 4) / 2 = 118 each, flush to the edges.
        assert_eq!(footer(240, 180), [[0, 153, 118, 20], [122, 153, 118, 20]]);
    }

    #[test]
    fn a_click_on_the_id_or_the_version_copies_it() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let mut screen =
            ModsScreen::new(427, 240, false, &mods(&[installed("zoom", "Zoom", None)]));
        let mut copied = Vec::new();
        screen.select(screen.entries.iter().position(|e| e.id == "zoom").unwrap());
        let g = screen.geo();
        let e = screen.current().unwrap().clone();
        let (items, ..) = screen.panel_items(&m, &e);
        for placed in &items {
            if let Kind::Subtitle(pieces) = &placed.kind {
                for piece in pieces {
                    let at = [g.panel[0] + piece.x, g.panel[1] + placed.y, piece.w, 9];
                    let (x, y) = centre(at);
                    copied.push(screen.click(&m, x, y).0);
                }
            }
        }
        assert_eq!(
            copied,
            [Action::Copy("zoom".into()), Action::Copy("1.0".into())]
        );
    }

    #[test]
    fn the_arrow_keys_walk_the_list_and_scroll_it() {
        let list: Vec<_> = (0..20)
            .map(|i| installed(&format!("m{i:02}"), &format!("Mod {i:02}"), None))
            .collect();
        let mut screen = ModsScreen::new(427, 240, false, &mods(&list));
        screen.step(1);
        assert_eq!(screen.current().map(|e| e.name.as_str()), Some("Horizon"));
        for _ in 0..21 {
            screen.step(1);
        }
        assert_eq!(screen.current().map(|e| e.name.as_str()), Some("Mod 19"));
        assert!(screen.list.amount > 0.0);
        screen.step(1);
        assert_eq!(screen.current().map(|e| e.name.as_str()), Some("Mod 19"));
        screen.step(-30);
        assert_eq!(screen.current().map(|e| e.name.as_str()), Some("Horizon"));
        assert_eq!(screen.list.amount, 0.0);
    }

    #[test]
    fn a_stopped_or_unlisted_mod_says_so_in_its_panel() {
        let (mut broken, info) = installed("broken", "Broken", None);
        broken.problem = Some("Not loaded: boom".into());
        let screen = ModsScreen::new(427, 240, false, &mods(&[(broken, info)]));
        let note = screen
            .entries
            .iter()
            .find(|e| e.id == "broken")
            .and_then(|e| e.status.as_ref())
            .map(Text::plain);
        assert_eq!(note.as_deref(), Some("Not loaded: boom"));
        // Without a problem, a mod that is not on says that instead.
        let (idle, info) = installed("idle", "Idle", None);
        let screen = ModsScreen::new(427, 240, false, &mods(&[(idle, info)]));
        let idle = screen.entries.iter().find(|e| e.id == "idle").unwrap();
        assert!(idle.status.is_some());
    }

    #[test]
    fn a_loading_warning_shows_in_the_players_language_without_failing_the_row() {
        let (mut row, mut info) = installed("warns", "Warns", None);
        row.dev = true;
        info.warnings = vec![("test.warn.optional".into(), vec!["jei".into()])];
        let note = |language| {
            crate::lang::scoped(language, || {
                let screen =
                    ModsScreen::new(427, 240, false, &mods(&[(row.clone(), info.clone())]));
                let entry = screen.entries.iter().find(|e| e.id == "warns").unwrap();
                entry.note.as_ref().map(Text::plain)
            })
            .0
        };
        let english = crate::lang::Language::of(
            "en_us",
            [(
                "test.warn.optional".to_owned(),
                "No %s: recipes hidden".to_owned(),
            )],
        );
        assert_eq!(note(english).as_deref(), Some("No jei: recipes hidden"));
        let missing = crate::lang::Language::of("en_us", Vec::<(String, String)>::new());
        assert_eq!(note(missing).as_deref(), Some("test.warn.optional"));
    }

    #[test]
    fn all_loading_notices_remain_visible_beside_failed_and_off_status() {
        let language = crate::lang::Language::of(
            "en_us",
            [
                (
                    "horizon.mods.screen.off".to_owned(),
                    "Off for this launch".to_owned(),
                ),
                ("test.first".to_owned(), "First warning: %s".to_owned()),
                ("test.second".to_owned(), "Second override: %s".to_owned()),
            ],
        );
        let _ = crate::lang::scoped(language, || {
            for problem in [None, Some("Refused fixture")] {
                let (mut row, mut info) = installed("warns", "Warns", None);
                row.problem = problem.map(str::to_owned);
                info.warnings = vec![
                    ("test.first".into(), vec!["other".into()]),
                    ("test.second".into(), vec!["third".into()]),
                ];
                let screen = ModsScreen::new(427, 240, false, &mods(&[(row, info)]));
                let entry = screen
                    .entries
                    .iter()
                    .find(|entry| entry.id == "warns")
                    .unwrap();
                assert_eq!(
                    entry.status.as_ref().unwrap().plain(),
                    problem.unwrap_or("Off for this launch")
                );
                assert_eq!(
                    entry.note.as_ref().unwrap().plain(),
                    "First warning: other\n\nSecond override: third"
                );
            }
        });
    }

    #[test]
    fn scrolling_long_notices_keeps_the_selected_mod_and_failure_status_fixed() {
        let language = crate::lang::Language::of(
            "en_us",
            [
                (
                    "horizon.mods.screen.off".to_owned(),
                    "Off for this launch".to_owned(),
                ),
                ("test.long".to_owned(), "%s".to_owned()),
            ],
        );
        let _ = crate::lang::scoped(language, || {
            let atlas = atlas();
            let m = Measure(&atlas.font);
            for problem in [None, Some("Refused fixture")] {
                let (mut row, mut info) = installed("warns", "Warns", None);
                row.problem = problem.map(str::to_owned);
                info.warnings = vec![(
                    "test.long".into(),
                    vec!["A complete warning sentence. ".repeat(100)],
                )];
                let mut screen = ModsScreen::new(427, 240, false, &mods(&[(row, info)]));
                screen.select(
                    screen
                        .entries
                        .iter()
                        .position(|entry| entry.id == "warns")
                        .unwrap(),
                );
                let [px, py, _, view] = screen.geo().panel;
                let at = |draws: &[Draw], expected: &str| {
                    draws.iter().find_map(|draw| match draw {
                        Draw::Text { text, x, y, .. } if *x >= px && text.plain() == expected => {
                            Some((*x, *y))
                        }
                        _ => None,
                    })
                };
                let before = screen.draw(&m, (0, 0), 0, 0);
                assert!(screen.panel_content.get() > view);
                screen.scroll(f64::from(px + 20), f64::from(py + view - 1), -100.0);
                assert!(screen.panel.amount > 0.0);
                let after = screen.draw(&m, (0, 0), 0, 0);
                for text in [
                    "Warns",
                    "warns",
                    "1.0",
                    problem.unwrap_or("Off for this launch"),
                ] {
                    assert!(at(&before, text).is_some(), "missing header: {text}");
                    assert_eq!(
                        at(&before, text),
                        at(&after, text),
                        "scrolled header: {text}"
                    );
                }
            }
        });
    }

    #[test]
    fn a_tall_failure_status_leaves_the_whole_small_pane_scrollable() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let (mut row, mut info) = installed("failed", "Failed", None);
        row.problem = Some("A complete refusal sentence. ".repeat(12));
        info.warnings = vec![(
            "test.long".into(),
            vec!["An overflowing warning. ".repeat(100)],
        )];
        let mut screen = ModsScreen::new(427, 100, false, &mods(&[(row, info)]));
        screen.select(0);
        screen.draw(&m, (0, 0), 0, 0);
        assert_eq!(screen.panel_pinned.get(), 0);
        assert!(screen.panel_content.get() > screen.geo().panel[3]);
    }

    /// `BackgroundWithPipingWidget`: the info panel is framed like the list, 2 px beyond its
    /// 250 px contents on every side, with the list's background between two separators.
    #[test]
    fn the_info_panel_has_the_lists_background_and_separators() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let screen = ModsScreen::new(427, 240, false, &ModList::default());
        let [px, py, _, view] = screen.geo().panel;
        let draws = screen.draw(&m, (0, 0), 0, 0);
        let blit = |texture: &str, x: i32, y: i32, w: i32, h: i32| {
            draws.iter().any(|call| {
                matches!(call, Draw::Blit { texture: t, x: bx, y: by, w: bw, h: bh, .. }
                    if t == texture && (*bx, *by, *bw, *bh) == (x, y, w, h))
            })
        };
        assert!(blit(
            "textures/gui/menu_list_background.png",
            px - 2,
            py - 2,
            254,
            view + 4
        ));
        assert!(blit(
            "textures/gui/header_separator.png",
            px - 2,
            py - 2,
            254,
            2
        ));
        assert!(blit(
            "textures/gui/footer_separator.png",
            px - 2,
            py + view,
            254,
            2
        ));
    }

    /// The panel's frame reaches 5 px into the list's scrollbar at 427 px wide, so it is drawn
    /// first and the list, scrollbar included, goes over it.
    #[test]
    fn the_panel_frame_is_drawn_under_the_list() {
        let atlas = atlas();
        let m = Measure(&atlas.font);
        let screen = ModsScreen::new(427, 240, false, &ModList::default());
        let [px, _, _, _] = screen.geo().panel;
        let [lx, ..] = screen.geo().list;
        let draws = screen.draw(&m, (0, 0), 0, 0);
        let background_at = |x: i32| {
            draws.iter().position(|call| {
                matches!(call, Draw::Blit { texture, x: bx, .. }
                    if texture == "textures/gui/menu_list_background.png" && *bx == x)
            })
        };
        let (frame, list) = (background_at(px - 2).unwrap(), background_at(lx).unwrap());
        assert!(frame < list, "frame at {frame}, list at {list}");
    }
}
