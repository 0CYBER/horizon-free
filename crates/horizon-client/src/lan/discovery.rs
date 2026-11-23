//! LAN game discovery: LanServerPinger announces a hosted world on the
//! multicast group 224.0.2.60:4445 every 1.5 seconds as
//! `[MOTD]text[/MOTD][AD]port[/AD]`, and LanServerDetection collects those
//! announcements for the multiplayer screen.
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 2, 60);
pub const PORT: u16 = 4445;
/// LanServerPinger sleeps 1500 ms between announcements.
const INTERVAL: Duration = Duration::from_millis(1500);

/// LanServerPinger.createPingString.
pub fn ping_string(motd: &str, port: u16) -> String {
    ping_string_text(motd, &port.to_string())
}
/// LanServerPinger.createPingString with the address as text.
pub fn ping_string_text(motd: &str, address: &str) -> String {
    format!("[MOTD]{motd}[/MOTD][AD]{address}[/AD]")
}
/// String.indexOf(needle, from) on a Rust string (byte offsets).
fn index_of(s: &str, needle: &str, from: usize) -> Option<usize> {
    s.get(from..)?.find(needle).map(|i| i + from)
}
/// LanServerPinger.parseMotd; "missing no" when there is none.
pub fn parse_motd(s: &str) -> String {
    let Some(start) = s.find("[MOTD]") else {
        return "missing no".to_owned();
    };
    match index_of(s, "[/MOTD]", start + "[MOTD]".len()) {
        Some(end) => s[start + "[MOTD]".len()..end].to_owned(),
        None => "missing no".to_owned(),
    }
}
/// LanServerPinger.parseAddress: the text between [AD] and [/AD] after the
/// only [/MOTD], as announced (vanilla does not check it is a port).
pub fn parse_address(s: &str) -> Option<String> {
    let end_motd = s.find("[/MOTD]")?;
    if index_of(s, "[/MOTD]", end_motd + "[/MOTD]".len()).is_some() {
        return None;
    }
    let start = index_of(s, "[AD]", end_motd + "[/MOTD]".len())?;
    let end = index_of(s, "[/AD]", start + "[AD]".len())?;
    Some(s[start + "[AD]".len()..end].to_owned())
}
/// The most games the list keeps: any host on the network can announce, and the
/// multiplayer screen copies the list every frame. Vanilla's list has no bound;
/// this one only matters past 64 distinct announcements.
pub const MAX_GAMES: usize = 64;
/// The port a LAN announcement's address names: ASCII digits that make a port
/// from 1 to 65535, or 25565 when it is empty. Anything else names none.
pub fn announced_port(address: &str) -> Option<u16> {
    if address.is_empty() {
        return Some(25565);
    }
    if !address.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    address.parse::<u16>().ok().filter(|&port| port != 0)
}
/// LanServerDetection.LanServerList.addServer: an announcement from `ip`.
/// A new address joins the list; a known one keeps its first MOTD. True
/// when the list changed. The listed address is `ip:address` as announced,
/// even when it is empty or no port, as vanilla lists it. Joining goes to `ip`
/// and the [`announced_port`] (an empty address joins on 25565), so a LAN
/// announcement never points a join at another host; one that names no port
/// has nowhere to join. A full list takes no more.
pub fn add_server(games: &mut Vec<LanGame>, ping: &str, ip: &str) -> bool {
    let motd = parse_motd(ping);
    let Some(address) = parse_address(ping) else {
        return false;
    };
    if games.len() >= MAX_GAMES {
        return false;
    }
    let join = ip
        .parse::<IpAddr>()
        .ok()
        .zip(announced_port(&address))
        .map(|(ip, port)| SocketAddr::new(ip, port));
    let address = format!("{ip}:{address}");
    if games.iter().any(|g| g.address == address) {
        return false;
    }
    games.push(LanGame {
        motd,
        address,
        join,
    });
    true
}

/// Announces one hosted world until dropped.
pub struct Pinger {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Pinger {
    pub fn start(motd: String, port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let message = ping_string(&motd, port);
        let thread = std::thread::Builder::new()
            .name("horizon-lan-ping".into())
            .spawn(move || {
                let target = SocketAddr::V4(SocketAddrV4::new(GROUP, PORT));
                while !flag.load(Ordering::Relaxed) {
                    let _ = socket.send_to(message.as_bytes(), target);
                    let mut slept = Duration::ZERO;
                    while slept < INTERVAL && !flag.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(50));
                        slept += Duration::from_millis(50);
                    }
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Pinger {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A world announced on the local network.
#[derive(Clone, Debug, PartialEq)]
pub struct LanGame {
    pub motd: String,
    /// The address as vanilla lists it: the sender's IP, a colon and the
    /// announced text.
    pub address: String,
    /// Where joining it connects: the sender's IP and the announced port.
    /// None when the announcement names no port.
    pub join: Option<SocketAddr>,
}
/// The most characters of an address a LAN row shows.
const SHOWN_ADDRESS_CHARS: usize = 64;
impl LanGame {
    /// The address as its row shows it: control and bidirectional-control
    /// characters escaped as `\u{..}`, so announced text cannot reorder or
    /// break the row, and cut to [`SHOWN_ADDRESS_CHARS`] with an ellipsis.
    pub fn shown_address(&self) -> String {
        let mut shown = String::new();
        for (i, c) in self.address.chars().enumerate() {
            if i == SHOWN_ADDRESS_CHARS {
                shown.push('\u{2026}');
                break;
            }
            let bidi = matches!(
                c,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            );
            if c.is_control() || bidi {
                shown += &format!("\\u{{{:x}}}", c as u32);
            } else {
                shown.push(c);
            }
        }
        shown
    }
}
type Games = Arc<Mutex<Vec<LanGame>>>;

/// Listens for announcements until the last clone is dropped.
#[derive(Clone, Debug)]
pub struct Scanner {
    inner: Arc<ScanInner>,
}
#[derive(Debug)]
struct ScanInner {
    games: Games,
    stop: Arc<AtomicBool>,
    /// How many games `take_found` has handed out.
    taken: Mutex<usize>,
}
impl Drop for ScanInner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Scanner {
    /// Bind the discovery port; without it the list simply stays empty.
    pub fn start() -> Self {
        let games: Games = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        // Constructing multiplayer screens in unit tests must not open the LAN.
        if !cfg!(test)
            && !super::test_loopback()
            && let Ok(socket) = bind_listener()
        {
            let (games, stop) = (games.clone(), stop.clone());
            let _ = std::thread::Builder::new()
                .name("horizon-lan-scan".into())
                .spawn(move || listen(socket, games, stop));
        }
        Self {
            inner: Arc::new(ScanInner {
                games,
                stop,
                taken: Mutex::new(0),
            }),
        }
    }
    #[cfg(test)]
    pub(crate) fn for_tests(games: Vec<LanGame>) -> Self {
        Self {
            inner: Arc::new(ScanInner {
                games: Arc::new(Mutex::new(games)),
                stop: Arc::new(AtomicBool::new(false)),
                taken: Mutex::new(0),
            }),
        }
    }
    /// The worlds announced since the scan started, in order (vanilla's
    /// list keeps them while the multiplayer screen is open).
    pub fn games(&self) -> Vec<LanGame> {
        self.inner.games.lock().unwrap().clone()
    }
    /// `LanServerList.takeDirty` with `updateNetworkServers`' new count: the
    /// games found since the last call, with their place in [`games`](Self::games).
    pub fn take_found(&self) -> Vec<(usize, LanGame)> {
        let games = self.inner.games.lock().unwrap();
        let mut taken = self.inner.taken.lock().unwrap();
        let found = games.iter().cloned().enumerate().skip(*taken).collect();
        *taken = games.len();
        found
    }
}
fn bind_listener() -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(("0.0.0.0", PORT))?;
    socket.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    Ok(socket)
}
fn listen(socket: UdpSocket, games: Games, stop: Arc<AtomicBool>) {
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Relaxed) {
        let Ok((n, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        add_server(&mut games.lock().unwrap(), &text, &from.ip().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `takeDirty`: each world found is handed out once, in order.
    #[test]
    fn found_worlds_are_taken_once() {
        let scanner = Scanner {
            inner: Arc::new(ScanInner {
                games: Arc::default(),
                stop: Arc::default(),
                taken: Mutex::new(0),
            }),
        };
        let found = |s: &Scanner| -> Vec<usize> { s.take_found().iter().map(|f| f.0).collect() };
        assert!(found(&scanner).is_empty());
        add_server(
            &mut scanner.inner.games.lock().unwrap(),
            &ping_string("A", 1),
            "10.0.0.2",
        );
        add_server(
            &mut scanner.inner.games.lock().unwrap(),
            &ping_string("B", 2),
            "10.0.0.3",
        );
        assert_eq!(found(&scanner), [0, 1]);
        assert!(found(&scanner).is_empty());
        add_server(
            &mut scanner.inner.games.lock().unwrap(),
            &ping_string("C", 3),
            "10.0.0.4",
        );
        assert_eq!(scanner.take_found()[0].1.motd, "C");
    }

    /// Announcements from one host or many cannot grow the list past [`MAX_GAMES`].
    #[test]
    fn the_list_is_bounded() {
        let mut games = Vec::new();
        for i in 0..MAX_GAMES {
            assert!(add_server(
                &mut games,
                &ping_string("A", 1000 + i as u16),
                "10.0.0.2"
            ));
        }
        assert!(!add_server(
            &mut games,
            &ping_string("A", 60000),
            "10.0.0.2"
        ));
        assert_eq!(games.len(), MAX_GAMES);
    }

    /// LanServerList.addServer lists an announced address that is empty or no
    /// port as `ip:address`, like any other (the lan parity harness hears these).
    #[test]
    fn an_address_that_is_no_port_is_listed_as_announced() {
        let mut games = Vec::new();
        for (address, ip) in [
            ("", "192.168.1.10"),
            ("x", "10.0.0.2"),
            ("25565", "10.0.0.2"),
        ] {
            assert!(add_server(&mut games, &ping_string_text("A", address), ip));
        }
        let listed: Vec<&str> = games.iter().map(|g| g.address.as_str()).collect();
        assert_eq!(listed, ["192.168.1.10:", "10.0.0.2:x", "10.0.0.2:25565"]);
        assert!(!add_server(
            &mut games,
            &ping_string_text("B", ""),
            "192.168.1.10"
        ));
    }

    /// Joining goes to the sender's IP and a plain port, never to a host the
    /// announced text names.
    #[test]
    fn joining_goes_to_the_sender_and_a_plain_port() {
        for (address, port) in [
            ("", Some(25565)),
            ("1", Some(1)),
            ("65535", Some(65535)),
            ("0080", Some(80)),
            ("0", None),
            ("65536", None),
            ("+80", None),
            ("-1", None),
            (" 80", None),
            ("x", None),
            ("80:evil.example", None),
            ("evil.example:80", None),
            ("1:2", None),
            ("\u{0661}", None),
        ] {
            assert_eq!(announced_port(address), port, "{address:?}");
            let mut games = Vec::new();
            assert!(add_server(
                &mut games,
                &ping_string_text("A", address),
                "10.0.0.2"
            ));
            let expected = port.map(|p| SocketAddr::from(([10, 0, 0, 2], p)));
            assert_eq!(games[0].join, expected, "{address:?}");
        }
        let mut games = Vec::new();
        assert!(add_server(&mut games, &ping_string("A", 25566), "fe80::1"));
        assert_eq!(games[0].address, "fe80::1:25566");
        assert_eq!(games[0].join.unwrap().to_string(), "[fe80::1]:25566");
    }

    /// The row shows control and bidi characters escaped and a long address
    /// cut; the listed address keeps them.
    #[test]
    fn the_shown_address_is_escaped_and_cut() {
        let game = |address: &str| LanGame {
            motd: String::new(),
            address: address.to_owned(),
            join: None,
        };
        assert_eq!(game("10.0.0.2:25565").shown_address(), "10.0.0.2:25565");
        assert_eq!(
            game("10.0.0.2:\u{202e}x\n\u{2066}").shown_address(),
            "10.0.0.2:\\u{202e}x\\u{a}\\u{2066}"
        );
        let long = format!("10.0.0.2:{}", "9".repeat(100));
        let shown = game(&long).shown_address();
        assert_eq!(shown.chars().count(), SHOWN_ADDRESS_CHARS + 1);
        assert!(shown.ends_with('\u{2026}'));
        assert!(long.starts_with(shown.trim_end_matches('\u{2026}')));
    }

    #[test]
    fn ping_string_round_trips() {
        let s = ping_string("Steve - World", 25565);
        assert_eq!(s, "[MOTD]Steve - World[/MOTD][AD]25565[/AD]");
        assert_eq!(parse_motd(&s), "Steve - World");
        assert_eq!(parse_address(&s).as_deref(), Some("25565"));
        assert_eq!(parse_address("[AD]1[/AD]"), None);
        assert_eq!(parse_address("[/MOTD][AD]x[/AD][/MOTD]"), None);
        assert_eq!(parse_motd("junk"), "missing no");
        let mut games = Vec::new();
        assert!(add_server(&mut games, &s, "10.0.0.2"));
        assert!(!add_server(
            &mut games,
            &ping_string("Other", 25565),
            "10.0.0.2"
        ));
        assert_eq!(games[0].motd, "Steve - World");
    }
}
