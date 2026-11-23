//! Servers page text helpers: byte sizes, uptime and a backup's date and kind for people.
use horizon_launcher::servers::Backup;

pub(super) fn bytes(n: u64) -> String {
    match n {
        0..1_000_000 => format!("{} kB", n.div_ceil(1000)),
        1_000_000..1_000_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        _ => format!("{:.2} GB", n as f64 / 1e9),
    }
}

/// A backup's date and kind from its name (`2026-10-19_11-28-57-manual.zip`, UTC), for people.
/// Any other zip in the server's `backups/` (one put there by hand) is shown by its name.
pub(super) fn backup_label(b: &Backup) -> (String, String) {
    let stem = b.name.trim_end_matches(".zip");
    let kind = stem.get(19..).and_then(|rest| rest.strip_prefix('-'));
    let (when, kind) = match (stem.get(..19), kind) {
        (Some(when), Some(kind)) if stamped(when) && !kind.is_empty() => (when, kind),
        _ => return (stem.to_string(), String::new()),
    };
    let (date, time) = when.split_once('_').unwrap_or((when, ""));
    let label = format!("{date} {} UTC", time.replace('-', ":"));
    let kind = match kind {
        "auto" => "Automatic".to_string(),
        "manual" => "Made by hand".to_string(),
        "before-update" => "Before an update".to_string(),
        // horizon-server writes the tag with dashes (before-v2026-10-19-10).
        k if k.starts_with("before-v")
            && k[8..].chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            format!("Before updating to {}", k[7..].replace('-', "."))
        }
        k if k.starts_with("before-") => format!("Before updating to {}", &k[7..]),
        k => k.to_string(),
    };
    (label, kind)
}

/// Whether `when` is the time horizon-server names a backup by: `2026-10-19_11-28-57`.
fn stamped(when: &str) -> bool {
    when.bytes().enumerate().all(|(i, b)| match i {
        4 | 7 | 13 | 16 => b == b'-',
        10 => b == b'_',
        _ => b.is_ascii_digit(),
    })
}

pub(super) fn uptime(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h {:02} min", secs / 3600, secs / 60 % 60),
        _ => format!("{} d {} h", secs / 86_400, secs / 3600 % 24),
    }
}
