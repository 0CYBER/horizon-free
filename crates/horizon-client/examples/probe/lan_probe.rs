//! Horizon's LAN world discovery for tools/test_lan_parity.py: the cases of
//! VanillaLanOracle (announcements sent, datagrams heard, list resets) run
//! through `lan::discovery`, printing the same lines.
use horizon_client::lan::discovery::{
    LanGame, add_server, parse_address, parse_motd, ping_string_text,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let cases = std::fs::read_to_string(&args[1]).unwrap();
    let mut out = String::new();
    let mut games: Vec<LanGame> = Vec::new();
    for line in cases.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "ping" => out += &format!("ping {}\n", hex(ping_string_text(f[1], f[2]).as_bytes())),
            "heard" => {
                let received = String::from_utf8_lossy(&unhex(f[2])).into_owned();
                out += &format!(
                    "heard motd={} address={}\n",
                    parse_motd(&received),
                    parse_address(&received).unwrap_or_else(|| "null".into())
                );
                if add_server(&mut games, &received, f[1]) {
                    out += "list";
                    for g in &games {
                        out += &format!(" [{} @ {}]", g.motd, g.address);
                    }
                    out += "\n";
                }
            }
            "reset" => games.clear(),
            other => panic!("{other}"),
        }
    }
    std::fs::write(&args[2], out).unwrap();
}
