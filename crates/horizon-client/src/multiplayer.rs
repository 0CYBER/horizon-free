//! Multiplayer screens on the title: the server list (JoinMultiplayerScreen,
//! stored in servers.dat), Add/Edit Server, Direct Connection, and the
//! Disconnected screen. Entries are pinged in the background like
//! ServerStatusPinger to show the MOTD, player count and latency.
use crate::hud::{HudAtlas, HudBatch, rgba};
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
    Disconnected {
        reason: String,
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

impl Screen {
    pub fn list(saves: &Path) -> Self {
        let servers = servers::load(&servers_file(saves));
        let pings: Pings = Arc::default();
        ping_all(&servers, &pings);
        Self::List {
            servers,
            selected: None,
            pings,
        }
    }
    pub fn buttons(&self, w: f32, h: f32) -> Vec<ButtonSpec> {
        let b = |x: f32, y: f32, bw: f32, label: &str, enabled: bool, id: u8| {
            (x, y, bw, label.to_owned(), enabled, id)
        };
        match self {
            // JoinMultiplayerScreen's two button rows.
            Self::List { selected, .. } => vec![
                b(
                    w / 2.0 - 154.0,
                    h - 52.0,
                    100.0,
                    "Join Server",
                    selected.is_some(),
                    40,
                ),
                b(
                    w / 2.0 - 50.0,
                    h - 52.0,
                    100.0,
                    "Direct Connection",
                    true,
                    41,
                ),
                b(w / 2.0 + 54.0, h - 52.0, 100.0, "Add Server", true, 42),
                b(
                    w / 2.0 - 154.0,
                    h - 28.0,
                    74.0,
                    "Edit",
                    selected.is_some(),
                    43,
                ),
                b(
                    w / 2.0 - 76.0,
                    h - 28.0,
                    74.0,
                    "Delete",
                    selected.is_some(),
                    44,
                ),
                b(w / 2.0 + 2.0, h - 28.0, 74.0, "Refresh", true, 45),
                b(w / 2.0 + 80.0, h - 28.0, 74.0, "Back", true, 46),
            ],
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
            Self::Disconnected { .. } => vec![b(
                w / 2.0 - 100.0,
                h / 2.0 + 30.0,
                200.0,
                "Back to Server List",
                true,
                54,
            )],
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
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 120));
        let centered = |batch: &mut HudBatch, text: &str, y: f32, color: u32| {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, y, color, true);
        };
        match self {
            Self::List {
                servers,
                selected,
                pings,
            } => {
                centered(batch, "Play Multiplayer", 8.0, u32::MAX);
                batch.fill(atlas, 0.0, 32.0, w, h - 32.0 - 64.0, rgba(0, 0, 0, 100));
                let pings = pings.lock().unwrap();
                for (i, server) in servers.iter().enumerate() {
                    let y = 36.0 + i as f32 * 36.0;
                    if y + 36.0 > h - 64.0 {
                        break;
                    }
                    let x = w / 2.0 - 152.0;
                    if *selected == Some(i) {
                        batch.fill(
                            atlas,
                            x - 2.0,
                            y - 2.0,
                            306.0,
                            36.0,
                            rgba(128, 128, 128, 255),
                        );
                        batch.fill(atlas, x - 1.0, y - 1.0, 304.0, 34.0, rgba(0, 0, 0, 255));
                    }
                    batch.sprite_scaled(atlas, "misc/unknown_server", x, y, 32.0, 32.0, u32::MAX);
                    batch.text(atlas, &server.name, x + 35.0, y + 1.0, u32::MAX, false);
                    let (line, right, color) = match pings.get(&server.ip) {
                        Some(Ping::Ok {
                            motd, players, ms, ..
                        }) => (
                            motd.lines().next().unwrap_or("").to_owned(),
                            format!("{players}  {ms} ms"),
                            rgba(128, 128, 128, 255),
                        ),
                        Some(Ping::Failed(_)) => (
                            "Can't connect to server".to_owned(),
                            String::new(),
                            rgba(170, 0, 0, 255),
                        ),
                        _ => (
                            "Pinging...".to_owned(),
                            String::new(),
                            rgba(128, 128, 128, 255),
                        ),
                    };
                    batch.text(atlas, &line, x + 35.0, y + 12.0, color, false);
                    batch.text(
                        atlas,
                        &server.ip,
                        x + 35.0,
                        y + 22.0,
                        rgba(96, 96, 96, 255),
                        false,
                    );
                    if !right.is_empty() {
                        let tw = HudBatch::text_width(atlas, &right);
                        batch.text(
                            atlas,
                            &right,
                            x + 300.0 - tw,
                            y + 1.0,
                            rgba(128, 128, 128, 255),
                            false,
                        );
                    }
                }
                if servers.is_empty() {
                    centered(
                        batch,
                        "No servers yet: add one or use Direct Connection",
                        h / 2.0 - 20.0,
                        rgba(160, 160, 160, 255),
                    );
                }
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
            Self::Disconnected { reason } => {
                centered(
                    batch,
                    "Connection Lost",
                    h / 2.0 - 40.0,
                    rgba(170, 170, 170, 255),
                );
                for (i, line) in wrap(reason, 300.0, atlas).iter().enumerate() {
                    centered(batch, line, h / 2.0 - 16.0 + i as f32 * 9.0, u32::MAX);
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
                    servers, selected, ..
                },
                Some(40),
            ) => {
                if let Some(s) = selected.and_then(|i| servers.get(i)) {
                    return Outcome::Connect(s.ip.clone());
                }
            }
            (Self::List { .. }, Some(41)) => {
                *self = Self::Direct {
                    address: String::new(),
                }
            }
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
                    servers, selected, ..
                },
                None,
            ) => {
                let index = ((y - 36.0) / 36.0).floor();
                if y >= 36.0
                    && y < h - 64.0
                    && (x - w / 2.0).abs() < 154.0
                    && (index as usize) < servers.len()
                {
                    let index = index as usize;
                    if *selected == Some(index) {
                        return Outcome::Connect(servers[index].ip.clone());
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
                Self::Edit { .. } | Self::Direct { .. } | Self::Disconnected { .. },
                Some(51 | 53 | 54),
            ) => *self = Self::list(saves),
            (Self::Direct { address }, Some(52)) => {
                return Outcome::Connect(address.trim().to_owned());
            }
            (Self::Edit { focus, .. }, None) => {
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
    fn focused_field(&mut self) -> Option<&mut String> {
        match self {
            Self::Edit {
                name,
                address,
                focus,
                ..
            } => Some(if *focus == 0 { name } else { address }),
            Self::Direct { address } => Some(address),
            _ => None,
        }
    }
    pub fn type_text(&mut self, text: &str) {
        if let Some(field) = self.focused_field() {
            for c in text.chars().filter(|c| !c.is_control()) {
                if field.chars().count() < 128 {
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
        if let Self::Edit { focus, .. } = self {
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
