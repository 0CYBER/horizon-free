//! The Rust side of tools/test_server_files_parity.py: reads each case
//! directory the way VanillaServerFilesOracle does (server.properties,
//! eula.txt, the four user lists and scripted list changes) and prints the
//! same report. `server_files_probe CASES OUTPUT`.
use horizon_client::server::{
    eula,
    java_properties::now_millis,
    properties::{DIFFICULTIES, GAME_MODES, ServerProperties, SystemEntropy},
    user_lists::{BanEntry, BanList, IpBanList, NameAndId, OpEntry, OpList, WhiteEntry, WhiteList},
};
use horizon_protocol::crypto::uuid_string;
use std::{fmt::Write as _, path::Path};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = crate::args().skip(1);
    let root = std::path::PathBuf::from(args.next().ok_or("CASES OUTPUT")?);
    let output = args.next().ok_or("OUTPUT")?;
    let mut cases: Vec<_> = std::fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    let mut out = String::new();
    for dir in cases {
        writeln!(out, "== {}", dir.file_name().unwrap().to_string_lossy())?;
        run_case(&dir, &mut out)?;
    }
    std::fs::write(output, out)?;
    Ok(())
}

/// Files.readAllLines: split on \n, \r or \r\n.
fn lines(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_owned)
        .collect();
    if v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    v
}
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}
fn has_key(text: &str, key: &str) -> bool {
    text.split('\n')
        .any(|l| l.trim_matches(|c: char| c <= ' ').starts_with(key))
}

fn work(dir: &Path) -> std::io::Result<std::path::PathBuf> {
    let w = std::env::temp_dir().join(format!(
        "server-files-{}-{}",
        std::process::id(),
        dir.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&w);
    std::fs::create_dir_all(&w)?;
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        std::fs::copy(e.path(), w.join(e.file_name()))?;
    }
    Ok(w)
}

fn run_case(dir: &Path, out: &mut String) -> Result<(), Box<dyn std::error::Error>> {
    let w = work(dir)?;
    let props = w.join("server.properties");
    if dir.join("server.properties").exists() || dir.join("properties.default").exists() {
        let before = std::fs::read(&props)
            .map(|b| latin1(&b))
            .unwrap_or_default();
        match ServerProperties::from_file(&props, &mut SystemEntropy::default()) {
            Ok(p) => {
                print_properties(&p, &before, out)?;
                p.store(&props)?;
                writeln!(out, "stored:")?;
                for (i, line) in lines(&std::fs::read_to_string(&props)?)
                    .into_iter()
                    .enumerate()
                {
                    let line = if i == 1 && line.starts_with('#') {
                        "#DATE".to_owned()
                    } else if line.starts_with("management-server-secret=")
                        && !has_key(&before, "management-server-secret")
                    {
                        "management-server-secret=<random>".to_owned()
                    } else {
                        line
                    };
                    writeln!(out, "{line}")?;
                }
            }
            Err(e) => {
                let class = e.split(':').next().unwrap_or("");
                writeln!(out, "ERROR {class}")?;
            }
        }
    }
    if dir.join("eula.txt").exists() || dir.join("eula.missing").exists() {
        let path = w.join("eula.txt");
        let agreed = eula::agreed(&path).unwrap_or(false);
        writeln!(out, "eula agreed={agreed}")?;
        if let Ok(bytes) = std::fs::read(&path) {
            for (i, line) in lines(&latin1(&bytes)).into_iter().enumerate() {
                if i == 1 && line.starts_with('#') {
                    writeln!(out, "#DATE")?;
                } else {
                    writeln!(out, "{line}")?;
                }
            }
        }
    }
    let mut ops = OpList::new(w.join("ops.json"));
    let mut white = WhiteList::new(w.join("whitelist.json"));
    let mut bans = BanList::new(w.join("banned-players.json"));
    let mut ips = IpBanList::new(w.join("banned-ips.json"));
    let mut any = false;
    macro_rules! load {
        ($list:expr, $name:literal) => {
            if $list.file.exists() {
                any = true;
                if let Err(_) = $list.load() {
                    writeln!(
                        out,
                        "{} load ERROR com.google.gson.JsonSyntaxException",
                        $name
                    )?;
                }
            }
        };
    }
    load!(ops, "ops");
    load!(white, "whitelist");
    load!(bans, "banned-players");
    load!(ips, "banned-ips");
    if let Ok(actions) = std::fs::read_to_string(dir.join("actions.txt")) {
        any = true;
        for line in actions.lines().filter(|l| !l.trim().is_empty()) {
            let a: Vec<&str> = line.split('\t').collect();
            let user = |n: &str, u: &str| NameAndId {
                id: horizon_client::server::properties::parse_uuid(u).unwrap(),
                name: n.to_owned(),
            };
            fn ban<T>(
                user: T,
                created: &str,
                source: &str,
                expires: &str,
                reason: &str,
            ) -> BanEntry<T> {
                let date = |s: &str| (s != "-").then(|| s.parse::<i64>().unwrap());
                BanEntry {
                    user,
                    created: date(created).unwrap_or_else(now_millis),
                    source: if source != "-" {
                        source.to_owned()
                    } else {
                        "(Unknown)".into()
                    },
                    expires: date(expires),
                    reason: (reason != "-").then(|| reason.to_owned()),
                }
            }
            let r = match a[0] {
                "op" => ops.add(OpEntry {
                    user: user(a[1], a[2]),
                    level: a[3].parse::<i32>()?.clamp(0, 4),
                    bypasses_player_limit: a[4].eq_ignore_ascii_case("true"),
                }),
                "deop" => ops.remove(&user(a[1], a[2])),
                "white" => white.add(WhiteEntry {
                    user: user(a[1], a[2]),
                }),
                "unwhite" => white.remove(&user(a[1], a[2])),
                "ban" => bans.add(ban(user(a[1], a[2]), a[3], a[4], a[5], a[6])),
                "pardon" => bans.remove(&user(a[1], a[2])),
                "banip" => ips.add(ban(a[1].to_owned(), a[2], a[3], a[4], a[5])),
                "pardonip" => ips.remove(&a[1].to_owned()),
                "isbanned" => bans.contains(&user(a[1], a[2])),
                "isop" => ops.get(&user(a[1], a[2])).is_some(),
                other => return Err(format!("unknown action {other}").into()),
            };
            writeln!(out, "action {} {r}", a[0])?;
        }
    }
    if any {
        for (name, json) in [
            ("ops", ops.to_json()),
            ("whitelist", white.to_json()),
            ("banned-players", bans.to_json()),
            ("banned-ips", ips.to_json()),
        ] {
            writeln!(out, "{name}:\n{json}")?;
        }
        let mut names: Vec<String> = ops.entries().iter().map(|e| e.user.name.clone()).collect();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        writeln!(out, "op users {}", names.join(","))?;
        let mut names: Vec<String> = white
            .entries()
            .iter()
            .map(|e| e.user.name.clone())
            .collect();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        writeln!(out, "white users {}", names.join(","))?;
    }
    let _ = std::fs::remove_dir_all(&w);
    Ok(())
}

fn print_properties(p: &ServerProperties, before: &str, out: &mut String) -> std::fmt::Result {
    let secret = if has_key(before, "management-server-secret") {
        p.management_server_secret.clone()
    } else {
        "<random>".into()
    };
    let pack = p.resource_pack.as_ref().map_or_else(
        || "none".to_owned(),
        |i| {
            format!(
                "{} {} {} {} {}",
                uuid_string(i.id),
                i.url,
                i.hash,
                i.required,
                i.prompt.is_some()
            )
        },
    );
    let opt_bool = |b: Option<bool>| b.map_or_else(|| "null".to_owned(), |b| b.to_string());
    let rows: Vec<(&str, String)> = vec![
        ("online-mode", p.online_mode.to_string()),
        (
            "prevent-proxy-connections",
            p.prevent_proxy_connections.to_string(),
        ),
        ("server-ip", p.server_ip.clone()),
        ("allow-flight", p.allow_flight.to_string()),
        ("motd", p.motd.clone()),
        ("enable-code-of-conduct", p.code_of_conduct.to_string()),
        ("bug-report-link", p.bug_report_link.clone()),
        ("force-gamemode", p.force_game_mode.to_string()),
        ("enforce-whitelist", p.enforce_whitelist.to_string()),
        ("difficulty", DIFFICULTIES[p.difficulty as usize].to_owned()),
        ("gamemode", GAME_MODES[p.game_mode as usize].to_owned()),
        ("level-name", p.level_name.clone()),
        ("server-port", p.server_port.to_string()),
        (
            "management-server-enabled",
            p.management_server_enabled.to_string(),
        ),
        ("management-server-host", p.management_server_host.clone()),
        (
            "management-server-port",
            p.management_server_port.to_string(),
        ),
        ("management-server-secret", secret),
        (
            "management-server-tls-enabled",
            p.management_server_tls_enabled.to_string(),
        ),
        (
            "management-server-tls-keystore",
            p.management_server_tls_keystore.clone(),
        ),
        (
            "management-server-tls-keystore-password",
            p.management_server_tls_keystore_password.clone(),
        ),
        (
            "management-server-allowed-origins",
            p.management_server_allowed_origins.clone(),
        ),
        (
            "announce-player-achievements",
            opt_bool(p.announce_player_achievements),
        ),
        ("enable-query", p.enable_query.to_string()),
        ("query.port", p.query_port.to_string()),
        ("enable-rcon", p.enable_rcon.to_string()),
        ("rcon.port", p.rcon_port.to_string()),
        ("rcon.password", p.rcon_password.clone()),
        ("hardcore", p.hardcore.to_string()),
        ("use-native-transport", p.use_native_transport.to_string()),
        ("spawn-protection", p.spawn_protection.to_string()),
        ("op-permission-level", p.op_permission_level.to_string()),
        (
            "function-permission-level",
            p.function_permission_level.to_string(),
        ),
        ("max-tick-time", p.max_tick_time.to_string()),
        (
            "max-chained-neighbor-updates",
            p.max_chained_neighbor_updates.to_string(),
        ),
        ("rate-limit", p.rate_limit.to_string()),
        (
            "command-spam-threshold-seconds",
            p.command_spam_threshold_seconds.to_string(),
        ),
        (
            "chat-spam-threshold-seconds",
            p.chat_spam_threshold_seconds.to_string(),
        ),
        ("view-distance", p.view_distance.to_string()),
        ("simulation-distance", p.simulation_distance.to_string()),
        ("max-players", p.max_players.to_string()),
        (
            "network-compression-threshold",
            p.network_compression_threshold.to_string(),
        ),
        ("broadcast-rcon-to-ops", p.broadcast_rcon_to_ops.to_string()),
        (
            "broadcast-console-to-ops",
            p.broadcast_console_to_ops.to_string(),
        ),
        ("max-world-size", p.max_world_size.to_string()),
        ("sync-chunk-writes", p.sync_chunk_writes.to_string()),
        ("region-file-compression", p.region_file_compression.clone()),
        ("enable-jmx-monitoring", p.enable_jmx_monitoring.to_string()),
        ("enable-status", p.enable_status.to_string()),
        ("hide-online-players", p.hide_online_players.to_string()),
        (
            "entity-broadcast-range-percentage",
            p.entity_broadcast_range_percentage.to_string(),
        ),
        ("text-filtering-config", p.text_filtering_config.clone()),
        (
            "text-filtering-version",
            p.text_filtering_version.to_string(),
        ),
        ("player-idle-timeout", p.player_idle_timeout.to_string()),
        (
            "status-heartbeat-interval",
            p.status_heartbeat_interval.to_string(),
        ),
        ("white-list", p.white_list.to_string()),
        (
            "enforce-secure-profile",
            p.enforce_secure_profile.to_string(),
        ),
        ("log-ips", p.log_ips.to_string()),
        (
            "pause-when-empty-seconds",
            p.pause_when_empty_seconds.to_string(),
        ),
        ("accepts-transfers", p.accepts_transfers.to_string()),
        (
            "seed",
            if p.random_seed {
                "<random>".into()
            } else {
                p.seed.to_string()
            },
        ),
        ("generate-structures", p.generate_structures.to_string()),
        ("resource-pack", pack),
        ("initial-enabled-packs", p.initial_enabled_packs.join("|")),
        ("initial-disabled-packs", p.initial_disabled_packs.join("|")),
    ];
    for (k, v) in rows {
        writeln!(out, "{k}={v}")?;
    }
    Ok(())
}
