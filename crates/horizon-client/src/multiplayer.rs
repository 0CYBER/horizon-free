//! Multiplayer screens on the title: the server list (JoinMultiplayerScreen,
//! stored in servers.dat), Add/Edit Server, Direct Connection, and the
//! Disconnected screen. Entries are pinged in the background like
//! ServerStatusPinger to show the MOTD, player count and latency.
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::lan::discovery::Scanner;
use horizon_protocol::servers::{self, ServerEntry};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// The result of pinging one address.
#[derive(Clone, Debug)]
pub enum Ping {
    Pinging,
    Ok {
        motd: String,
        players: String,
        ms: u128,
        version: String,
    },
    Failed(String),
}
type Pings = Arc<Mutex<HashMap<String, Ping>>>;

#[derive(Clone, Debug)]
pub enum Screen {
    List {
        servers: Vec<ServerEntry>,
        selected: Option<usize>,
        pings: Pings,
        /// Worlds announced on the local network, listed after the servers.
        lan: Scanner,
        scroll: crate::ui::Scroll,
    },
    /// Add (index None) or edit a server entry.
    Edit {
        index: Option<usize>,
        name: String,
        address: String,
        focus: u8,
    },
    Direct {
        address: String,
    },
    /// Join a friend's world with its invite code (Horizon's own, reached
    /// from Horizon's multiplayer page).
    Friend {
        code: String,
        /// The relay for short codes (optional; saved on Join).
        relay: String,
        focus: u8,
    },
    Disconnected {
        reason: String,
        /// Auto-reconnect's next attempt, until it is cancelled.
        retry: Option<crate::reconnect::Retry>,
    },
}
/// What a click asks the title screen to do.
pub enum Outcome {
    None,
    Back,
    Connect(String),
}
/// A button: x, y, width, label, enabled, id.
pub type ButtonSpec = (f32, f32, f32, String, bool, u8);

pub fn servers_file(saves: &Path) -> PathBuf {
    saves
        .parent()
        .map_or_else(|| PathBuf::from("servers.dat"), |p| p.join("servers.dat"))
}
fn ping_all(servers: &[ServerEntry], pings: &Pings) {
    for s in servers {
        let address = s.ip.clone();
        pings.lock().unwrap().insert(address.clone(), Ping::Pinging);
        let pings = pings.clone();
        let _ = std::thread::Builder::new()
            .name("horizon-ping".into())
            .spawn(move || {
                let result = match crate::net::ping(&address, Duration::from_secs(5)) {
                    Ok((json, ms)) => {
                        let lang = Default::default();
                        let motd = crate::net::json_text(&json["description"], &lang);
                        let players = format!(
                            "{}/{}",
                            json["players"]["online"].as_i64().unwrap_or(0),
                            json["players"]["max"].as_i64().unwrap_or(0)
                        );
                        let version = json["version"]["name"].as_str().unwrap_or("?").to_owned();
                        Ping::Ok {
                            motd,
                            players,
                            ms,
                            version,
                        }
                    }
                    Err(e) => Ping::Failed(e),
                };
                pings.lock().unwrap().insert(address, result);
            });
    }
}

/// The address of list row `index`: the saved servers, then the LAN games.
fn entry_address(servers: &[ServerEntry], lan: &Scanner, index: usize) -> Option<String> {
    match servers.get(index) {
        Some(s) => Some(s.ip.clone()),
        None => lan
            .games()
            .into_iter()
            .nth(index - servers.len())
            .map(|g| g.address),
    }
}

/// JoinMultiplayerScreen's `HeaderAndFooterLayout(33, 60)`.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 60.0;
/// ServerSelectionList: rows 305 wide and 36 high.
const ROW_WIDTH: f32 = 305.0;
const ROW_HEIGHT: f32 = 36.0;
/// One row of the list: the saved servers, the LAN header, the LAN games.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Server(usize),
    LanHeader,
    Lan(usize),
}
/// The list's top and bottom and its rows' left edge (`getRowLeft`).
fn list_area(w: f32, h: f32) -> (f32, f32, f32) {
    (
        HEADER,
        h - FOOTER,
        (w / 2.0).floor() - (ROW_WIDTH / 2.0).floor(),
    )
}
fn rows(servers: usize, lan: usize) -> impl Iterator<Item = Row> {
    (0..servers)
        .map(Row::Server)
        .chain(std::iter::once(Row::LanHeader))
        .chain((0..lan).map(Row::Lan))
}
/// `contentHeight`: the rows and 4 pixels of padding.
fn content_height(servers: usize, lan: usize) -> f32 {
    (servers + 1 + lan) as f32 * ROW_HEIGHT + 4.0
}
/// The row under `(x, y)` (`getEntryAtPosition`), scrolled by `scroll`.
fn row_at(
    servers: usize,
    lan: usize,
    (x, y): (f32, f32),
    size: (f32, f32),
    scroll: f32,
) -> Option<Row> {
    let (top, bottom, left) = list_area(size.0, size.1);
    if y < top || y >= bottom || x < left || x >= left + ROW_WIDTH {
        return None;
    }
    let index = ((y - top - 2.0 + scroll.trunc()) / ROW_HEIGHT).floor();
    if index < 0.0 {
        return None;
    }
    rows(servers, lan).nth(index as usize)
}
/// `LoadingDotsText.get`.
fn loading_dots(millis: u128) -> &'static str {
    ["O o o", "o O o", "o o O", "o O o"][(millis / 300 % 4) as usize]
}

impl Screen {
    /// The address of list row `row` (saved servers, then LAN games).
    pub fn address_of(&self, row: usize) -> Option<String> {
        match self {
            Self::List { servers, lan, .. } => entry_address(servers, lan, row),
            _ => None,
        }
    }
    pub fn list(saves: &Path) -> Self {
        let servers = servers::load(&servers_file(saves));
        let pings: Pings = Arc::default();
        ping_all(&servers, &pings);
        Self::List {
            servers,
            selected: None,
            pings,
            lan: Scanner::start(),
            scroll: crate::ui::Scroll::default(),
        }
    }
    pub fn buttons(&self, w: f32, h: f32) -> Vec<ButtonSpec> {
        let b = |x: f32, y: f32, bw: f32, label: &str, enabled: bool, id: u8| {
            (x, y, bw, label.to_owned(), enabled, id)
        };
        match self {
            // JoinMultiplayerScreen's footer: two rows of buttons, 4 apart,
            // centred in the 60-pixel footer. Join needs a server or LAN game
            // selected (onSelectedChange), Edit and Delete a saved server.
            Self::List {
                servers, selected, ..
            } => {
                let (w, h) = (w.ceil(), h.ceil());
                let x = ((w - 308.0) / 2.0).floor();
                let saved = selected.is_some_and(|i| i < servers.len());
                let label = |key: &str, fallback: &str| crate::lang::get_or(key, fallback);
                vec![
                    b(
                        x,
                        h - 52.0,
                        100.0,
                        &label("selectServer.select", "Join Server"),
                        selected.is_some(),
                        40,
                    ),
                    b(
                        x + 104.0,
                        h - 52.0,
                        100.0,
                        &label("selectServer.direct", "Direct Connection"),
                        true,
                        41,
                    ),
                    b(
                        x + 208.0,
                        h - 52.0,
                        100.0,
                        &label("selectServer.add", "Add Server"),
                        true,
                        42,
                    ),
                    b(
                        x,
                        h - 28.0,
                        74.0,
                        &label("selectServer.edit", "Edit"),
                        saved,
                        43,
                    ),
                    b(
                        x + 78.0,
                        h - 28.0,
                        74.0,
                        &label("selectServer.delete", "Delete"),
                        saved,
                        44,
                    ),
                    b(
                        x + 156.0,
                        h - 28.0,
                        74.0,
                        &label("selectServer.refresh", "Refresh"),
                        true,
                        45,
                    ),
                    b(
                        x + 234.0,
                        h - 28.0,
                        74.0,
                        &label("gui.back", "Back"),
                        true,
                        46,
                    ),
                ]
            }
            Self::Edit { address, .. } => vec![
                b(
                    w / 2.0 - 100.0,
                    h / 4.0 + 96.0 + 18.0,
                    200.0,
                    "Done",
                    servers::parse_address(address).is_some(),
                    50,
                ),
                b(
                    w / 2.0 - 100.0,
                    h / 4.0 + 120.0 + 18.0,
                    200.0,
                    "Cancel",
                    true,
                    51,
                ),
            ],
            Self::Direct { address } => vec![
                b(
                    w / 2.0 - 100.0,
                    h / 4.0 + 96.0 + 12.0,
                    200.0,
                    "Join Server",
                    servers::parse_address(address).is_some(),
                    52,
                ),
                b(
                    w / 2.0 - 100.0,
                    h / 4.0 + 120.0 + 12.0,
                    200.0,
                    "Cancel",
                    true,
                    53,
                ),
            ],
            Self::Friend { code, .. } => vec![
                b(
                    w / 2.0 - 100.0,
                    h / 4.0 + 96.0 + 18.0,
                    200.0,
                    "Join",
                    horizon_friends::invite::parse(code).is_some(),
                    56,
                ),
                b(
                    w / 2.0 - 100.0,
                    h / 4.0 + 120.0 + 18.0,
                    200.0,
                    "Cancel",
                    true,
                    57,
                ),
            ],
            Self::Disconnected { retry, .. } => {
                let mut v = vec![b(
                    w / 2.0 - 100.0,
                    h / 2.0 + 30.0,
                    200.0,
                    "Back to Server List",
                    true,
                    54,
                )];
                if retry.is_some() {
                    v.push(b(
                        w / 2.0 - 100.0,
                        h / 2.0 + 54.0,
                        200.0,
                        "Cancel Reconnecting",
                        true,
                        55,
                    ));
                }
                v
            }
        }
    }
    fn field(
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        label: &str,
        value: &str,
        y: f32,
        focused: bool,
        blink: bool,
    ) {
        let w = batch.width;
        crate::widgets::target_as(
            [w / 2.0 - 100.0, y + 11.0, 200.0, 20.0],
            crate::widgets::Kind::Field,
        );
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
            if focused {
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
        let shown = if focused && blink {
            format!("{value}_")
        } else {
            value.to_owned()
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
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, blink: bool) {
        let (w, h) = (batch.width, batch.height);
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let centered = |batch: &mut HudBatch, text: &str, y: f32, color: u32| {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, y, color, true);
        };
        match self {
            Self::List {
                servers,
                selected,
                pings,
                lan,
                scroll,
            } => {
                let (w, h) = (w.ceil(), h.ceil());
                let title = crate::lang::get_or("multiplayer.title", "Play Multiplayer");
                // The header's StringWidget, centred in its 33 pixels.
                let tw = HudBatch::text_width(atlas, &title);
                let tx = ((w - tw.ceil()) / 2.0).floor();
                batch.text(
                    atlas,
                    &title,
                    tx,
                    ((HEADER - 9.0) / 2.0).floor(),
                    u32::MAX,
                    true,
                );
                let games = lan.games();
                let (top, bottom, left) = list_area(w, h);
                let content = content_height(servers.len(), games.len());
                let mut scroll = *scroll;
                scroll.clamp(top, bottom, content);
                // AbstractSelectionList.extractWidgetRenderState: background,
                // the rows clipped to the list, separators, scrollbar.
                crate::ui::list_background(batch, atlas, top, bottom, false);
                let first = batch.vertices.len();
                let pings = pings.lock().unwrap();
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis());
                let gray = rgba(128, 128, 128, 255);
                for (i, row) in rows(servers.len(), games.len()).enumerate() {
                    let y = top + 2.0 - scroll.amount.trunc() + i as f32 * ROW_HEIGHT;
                    if y + ROW_HEIGHT < top || y > bottom {
                        continue;
                    }
                    let index = match row {
                        Row::Server(k) => Some(k),
                        Row::Lan(k) => Some(servers.len() + k),
                        Row::LanHeader => None,
                    };
                    if index.is_some()
                        && y + ROW_HEIGHT / 2.0 > top
                        && y + ROW_HEIGHT / 2.0 < bottom
                    {
                        crate::widgets::target_as(
                            [left, y, ROW_WIDTH, ROW_HEIGHT],
                            crate::widgets::Kind::Row,
                        );
                    }
                    // extractSelection: a white outline round black (the list
                    // has focus once a row is clicked).
                    if index.is_some() && *selected == index {
                        batch.fill(atlas, left, y, ROW_WIDTH, ROW_HEIGHT, u32::MAX);
                        batch.fill(
                            atlas,
                            left + 1.0,
                            y + 1.0,
                            ROW_WIDTH - 2.0,
                            ROW_HEIGHT - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    // Entry content: 2 pixels in from the row.
                    let (cx, cy) = (left + 2.0, y + 2.0);
                    match row {
                        Row::Server(k) => {
                            let server = &servers[k];
                            batch.sprite_scaled(
                                atlas,
                                "misc/unknown_server",
                                cx,
                                cy,
                                32.0,
                                32.0,
                                u32::MAX,
                            );
                            batch.text(atlas, &server.name, cx + 35.0, cy + 1.0, u32::MAX, false);
                            // OnlineServerEntry: up to two lines of MOTD, the
                            // player count left of the status icon.
                            let (motd, status, motd_color, icon) = match pings.get(&server.ip) {
                                Some(Ping::Ok {
                                    motd, players, ms, ..
                                }) => (
                                    motd.clone(),
                                    players.clone(),
                                    gray,
                                    format!(
                                        "server_list/ping_{}",
                                        match ms {
                                            0..150 => 5,
                                            150..300 => 4,
                                            300..600 => 3,
                                            600..1000 => 2,
                                            _ => 1,
                                        }
                                    ),
                                ),
                                Some(Ping::Failed(_)) => (
                                    crate::lang::get_or(
                                        "multiplayer.status.cannot_connect",
                                        "Can't connect to server",
                                    ),
                                    String::new(),
                                    rgba(255, 0, 0, 255),
                                    "server_list/unreachable".to_owned(),
                                ),
                                _ => {
                                    // The pinging bars bounce, offset by the row.
                                    let frame = (millis / 100 + i as u128 * 2) & 7;
                                    let frame = if frame > 4 { 8 - frame } else { frame };
                                    (
                                        crate::lang::get_or(
                                            "multiplayer.status.pinging",
                                            "Pinging...",
                                        ),
                                        String::new(),
                                        gray,
                                        format!("server_list/pinging_{}", frame + 1),
                                    )
                                }
                            };
                            for (n, line) in wrap(&motd, ROW_WIDTH - 4.0 - 34.0, atlas)
                                .iter()
                                .take(2)
                                .enumerate()
                            {
                                batch.text(
                                    atlas,
                                    line,
                                    cx + 35.0,
                                    cy + 12.0 + 9.0 * n as f32,
                                    motd_color,
                                    false,
                                );
                            }
                            let icon_x = cx + ROW_WIDTH - 4.0 - 15.0;
                            batch.sprite_scaled(
                                atlas,
                                &format!("gui/sprites/{icon}"),
                                icon_x,
                                cy,
                                10.0,
                                8.0,
                                u32::MAX,
                            );
                            if !status.is_empty() {
                                let sw = HudBatch::text_width(atlas, &status).ceil();
                                batch.text(
                                    atlas,
                                    &status,
                                    icon_x - sw - 5.0,
                                    cy + 1.0,
                                    gray,
                                    false,
                                );
                            }
                        }
                        Row::LanHeader => {
                            // LANHeader's LoadingDotsWidget: the label, then
                            // the dots on its last line, both centred.
                            let label = crate::lang::get_or(
                                "lanServer.scanning",
                                "Scanning for games on your local network",
                            );
                            let middle = cx + ((ROW_WIDTH - 4.0) / 2.0).floor();
                            let lw = HudBatch::text_width(atlas, &label).ceil();
                            let lx = middle - (lw / 2.0).floor();
                            let centre = lx + (lw / 2.0).floor();
                            batch.text(atlas, &label, lx, cy + 2.0, u32::MAX, false);
                            let dots = loading_dots(millis);
                            let dw = HudBatch::text_width(atlas, dots).ceil();
                            batch.text(
                                atlas,
                                dots,
                                centre - (dw / 2.0).floor(),
                                cy + 28.0 - 9.0 - 2.0,
                                gray,
                                false,
                            );
                        }
                        Row::Lan(k) => {
                            let game = &games[k];
                            let header = crate::lang::get_or("lanServer.title", "LAN World");
                            batch.text(atlas, &header, cx + 35.0, cy + 1.0, u32::MAX, false);
                            batch.text(atlas, &game.motd, cx + 35.0, cy + 12.0, gray, false);
                            batch.text(atlas, &game.address, cx + 35.0, cy + 23.0, gray, false);
                        }
                    }
                }
                batch.clip_since(first, 0.0, top, w, bottom);
                crate::ui::separators(batch, atlas, top, bottom, false);
                crate::ui::scrollbar(
                    batch,
                    atlas,
                    left + ROW_WIDTH + 8.0,
                    top,
                    bottom,
                    content,
                    scroll.amount,
                );
            }
            Self::Edit {
                index,
                name,
                address,
                focus,
            } => {
                centered(
                    batch,
                    if index.is_some() {
                        "Edit Server Info"
                    } else {
                        "Add Server"
                    },
                    17.0,
                    u32::MAX,
                );
                Self::field(atlas, batch, "Server Name", name, 53.0, *focus == 0, blink);
                Self::field(
                    atlas,
                    batch,
                    "Server Address",
                    address,
                    94.0,
                    *focus == 1,
                    blink,
                );
            }
            Self::Direct { address } => {
                centered(batch, "Direct Connection", 20.0, u32::MAX);
                Self::field(
                    atlas,
                    batch,
                    "Server Address",
                    address,
                    100.0 - 11.0,
                    true,
                    blink,
                );
            }
            Self::Friend { code, relay, focus } => {
                centered(batch, "Join a Friend", 20.0, u32::MAX);
                Self::field(atlas, batch, "Invite Code", code, 53.0, *focus == 0, blink);
                Self::field(
                    atlas,
                    batch,
                    "Relay (optional)",
                    relay,
                    94.0,
                    *focus == 1,
                    blink,
                );
            }
            Self::Disconnected { reason, retry } => {
                centered(
                    batch,
                    "Connection Lost",
                    h / 2.0 - 40.0,
                    rgba(170, 170, 170, 255),
                );
                for (i, line) in wrap(reason, 300.0, atlas).iter().enumerate() {
                    centered(batch, line, h / 2.0 - 16.0 + i as f32 * 9.0, u32::MAX);
                }
                if let Some(r) = retry {
                    let text = r.text(std::time::Instant::now());
                    centered(batch, &text, h / 2.0 + 16.0, rgba(170, 170, 170, 255));
                }
            }
        }
    }
    pub fn click(
        &mut self,
        id: Option<u8>,
        x: f32,
        y: f32,
        size: (f32, f32),
        saves: &Path,
    ) -> Outcome {
        let (w, h) = size;
        match (&mut *self, id) {
            (
                Self::List {
                    servers,
                    selected,
                    lan,
                    ..
                },
                Some(40),
            ) => {
                if let Some(address) = selected.and_then(|i| entry_address(servers, lan, i)) {
                    return Outcome::Connect(address);
                }
            }
            (Self::List { .. }, Some(41)) => {
                *self = Self::Direct {
                    address: String::new(),
                }
            }
            (Self::List { .. }, Some(47)) => *self = Self::friend(),
            (Self::List { .. }, Some(42)) => {
                *self = Self::Edit {
                    index: None,
                    name: "Minecraft Server".into(),
                    address: String::new(),
                    focus: 1,
                }
            }
            (
                Self::List {
                    servers, selected, ..
                },
                Some(43),
            ) => {
                if let Some((i, s)) = selected.and_then(|i| servers.get(i).map(|s| (i, s.clone())))
                {
                    *self = Self::Edit {
                        index: Some(i),
                        name: s.name,
                        address: s.ip,
                        focus: 0,
                    };
                }
            }
            (
                Self::List {
                    servers, selected, ..
                },
                Some(44),
            ) => {
                if let Some(i) = selected.take()
                    && i < servers.len()
                {
                    servers.remove(i);
                    let _ = servers::save(&servers_file(saves), servers);
                }
            }
            (Self::List { .. }, Some(45)) => *self = Self::list(saves),
            (Self::List { .. }, Some(46)) => return Outcome::Back,
            (
                Self::List {
                    servers,
                    selected,
                    lan,
                    scroll,
                    ..
                },
                None,
            ) => {
                let (w, h) = (w.ceil(), h.ceil());
                let games = lan.games().len();
                let (top, bottom, left) = list_area(w, h);
                let content = content_height(servers.len(), games);
                if scroll.press((x, y), left + ROW_WIDTH + 8.0, top, bottom, content) {
                    return Outcome::None;
                }
                // Clicking a selected row again joins it; the LAN header
                // takes the click without anything to join.
                let index = match row_at(servers.len(), games, (x, y), (w, h), scroll.amount) {
                    Some(Row::Server(k)) => Some(k),
                    Some(Row::Lan(k)) => Some(servers.len() + k),
                    Some(Row::LanHeader) => {
                        *selected = None;
                        None
                    }
                    None => None,
                };
                if let Some(index) = index
                    && let Some(address) = entry_address(servers, lan, index)
                {
                    if *selected == Some(index) {
                        return Outcome::Connect(address);
                    }
                    *selected = Some(index);
                }
            }
            (
                Self::Edit {
                    index,
                    name,
                    address,
                    ..
                },
                Some(50),
            ) => {
                let mut list = servers::load(&servers_file(saves));
                let entry = ServerEntry::new(name.trim(), address.trim());
                match index.and_then(|i| list.get_mut(i)) {
                    Some(e) => {
                        e.name = entry.name;
                        e.ip = entry.ip;
                    }
                    None => list.push(entry),
                }
                let _ = servers::save(&servers_file(saves), &list);
                *self = Self::list(saves);
            }
            (
                Self::Edit { .. }
                | Self::Direct { .. }
                | Self::Friend { .. }
                | Self::Disconnected { .. },
                Some(51 | 53 | 54 | 57),
            ) => *self = Self::list(saves),
            (Self::Disconnected { retry, .. }, Some(55)) => *retry = None,
            (Self::Direct { address }, Some(52)) => {
                return Outcome::Connect(address.trim().to_owned());
            }
            (Self::Friend { code, relay, .. }, Some(56)) => {
                if horizon_friends::invite::parse(code).is_some() {
                    crate::net::save_friends_relay(relay.trim());
                    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
                    return Outcome::Connect(format!("{}{code}", crate::net::FRIEND));
                }
            }
            (Self::Edit { focus, .. } | Self::Friend { focus, .. }, None) => {
                for (i, fy) in [(0u8, 53.0 + 11.0), (1, 94.0 + 11.0)] {
                    if y >= fy && y < fy + 20.0 && (x - w / 2.0).abs() < 100.0 {
                        *focus = i;
                    }
                }
            }
            _ => {}
        }
        Outcome::None
    }
    /// The mouse wheel over the server list (`scrollRate`: half a row).
    pub fn wheel(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        if let Self::List {
            servers,
            lan,
            scroll,
            ..
        } = self
        {
            let (top, bottom, _) = list_area(size.0.ceil(), size.1.ceil());
            if cursor.1 >= top && cursor.1 < bottom {
                let content = content_height(servers.len(), lan.games().len());
                scroll.wheel(delta, ROW_HEIGHT / 2.0, top, bottom, content);
            }
        }
    }
    /// The pointer moved: a held scrollbar follows it.
    pub fn drag(&mut self, y: f32, size: (f32, f32)) {
        if let Self::List {
            servers,
            lan,
            scroll,
            ..
        } = self
        {
            let (top, bottom, _) = list_area(size.0.ceil(), size.1.ceil());
            scroll.drag(
                y,
                top,
                bottom,
                content_height(servers.len(), lan.games().len()),
            );
        }
    }
    pub fn release(&mut self) {
        if let Self::List { scroll, .. } = self {
            scroll.release();
        }
    }
    fn focused_field(&mut self) -> Option<&mut String> {
        match self {
            Self::Edit {
                name,
                address,
                focus,
                ..
            } => Some(if *focus == 0 { name } else { address }),
            Self::Direct { address } => Some(address),
            Self::Friend {
                code, relay, focus, ..
            } => Some(if *focus == 0 { code } else { relay }),
            _ => None,
        }
    }
    /// Join a friend, with the relay the player set before.
    pub fn friend() -> Self {
        Self::Friend {
            code: String::new(),
            relay: crate::net::friends_relay_setting().unwrap_or_default(),
            focus: 0,
        }
    }
    /// Auto-reconnect's attempt, once its countdown is over.
    pub fn due_retry(&mut self) -> Option<crate::reconnect::Retry> {
        let Self::Disconnected { retry, .. } = self else {
            return None;
        };
        if !retry.as_ref()?.due(std::time::Instant::now()) {
            return None;
        }
        retry.take()
    }
    /// The focused text field, its id and maximum length (copy and paste).
    pub fn focused_text(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        let id = match self {
            Self::Edit { focus: 0, .. } => ("server.name", 0),
            Self::Edit { .. } => ("server.address", 0),
            Self::Direct { .. } => ("server.direct", 0),
            Self::Friend { focus: 0, .. } => ("friend.code", 0),
            Self::Friend { .. } => ("friend.relay", 0),
            _ => return None,
        };
        let max = self.max_length();
        self.focused_field().map(|f| (id, f, max))
    }
    /// Invite codes with IPv6 and a relay run past a server address's 128.
    fn max_length(&self) -> usize {
        if matches!(self, Self::Friend { .. }) {
            256
        } else {
            128
        }
    }
    pub fn type_text(&mut self, text: &str) {
        let max = self.max_length();
        if let Some(field) = self.focused_field() {
            for c in text.chars().filter(|c| !c.is_control()) {
                if field.chars().count() < max {
                    field.push(c);
                }
            }
        }
    }
    pub fn backspace(&mut self) {
        if let Some(field) = self.focused_field() {
            field.pop();
        }
    }
    pub fn tab(&mut self) {
        if let Self::Edit { focus, .. } | Self::Friend { focus, .. } = self {
            *focus = (*focus + 1) % 2;
        }
    }
    /// Escape: back to the list, or out of it (true).
    pub fn escape(&mut self, saves: &Path) -> bool {
        match self {
            Self::List { .. } => true,
            _ => {
                *self = Self::list(saves);
                false
            }
        }
    }
}

/// Greedy word wrap to a pixel width.
pub fn wrap(text: &str, width: f32, atlas: &HudAtlas) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if HudBatch::text_width(atlas, &candidate) > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lan_header_follows_the_saved_servers() {
        let all: Vec<Row> = rows(2, 1).collect();
        assert_eq!(
            all,
            [Row::Server(0), Row::Server(1), Row::LanHeader, Row::Lan(0)]
        );
        // No saved servers: the list is the scanning header alone.
        assert_eq!(rows(0, 0).collect::<Vec<_>>(), [Row::LanHeader]);
        assert_eq!(content_height(0, 0), 40.0);
    }

    #[test]
    fn rows_sit_two_pixels_under_the_header_and_scroll_with_the_list() {
        // 427x240 GUI: the list spans 33..180, rows 305 wide from x 61.
        let size = (427.0, 240.0);
        assert_eq!(list_area(size.0, size.1), (33.0, 180.0, 61.0));
        assert_eq!(row_at(3, 0, (100.0, 35.0), size, 0.0), Some(Row::Server(0)));
        assert_eq!(row_at(3, 0, (100.0, 70.9), size, 0.0), Some(Row::Server(0)));
        assert_eq!(row_at(3, 0, (100.0, 71.0), size, 0.0), Some(Row::Server(1)));
        assert_eq!(
            row_at(3, 0, (100.0, 35.0), size, 36.0),
            Some(Row::Server(1))
        );
        assert_eq!(
            row_at(3, 0, (100.0, 143.0), size, 0.0),
            Some(Row::LanHeader)
        );
        assert_eq!(row_at(3, 0, (60.0, 40.0), size, 0.0), None);
        assert_eq!(row_at(3, 0, (100.0, 32.0), size, 0.0), None);
    }

    #[test]
    fn loading_dots_step_every_300_ms() {
        let frames: Vec<&str> = [0, 299, 300, 600, 900, 1200].map(loading_dots).to_vec();
        assert_eq!(
            frames,
            ["O o o", "O o o", "o O o", "o o O", "o O o", "O o o"]
        );
    }
}
