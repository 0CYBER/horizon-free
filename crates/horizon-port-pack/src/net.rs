//! The agents port-pack's downloads go through: the jars of an imported modpack come from the same
//! places as everything else, so they take the same proxy — and leave the same hosts alone.
//!
//! The launcher hands its own setting over (`set_proxy`, from `proxy.rs`); a run of the
//! `horizon-port-pack` command reads `HTTPS_PROXY`, `HTTP_PROXY` and `ALL_PROXY` instead. Either
//! way a host in `NO_PROXY`, or on this computer, is reached directly: a host that only answers
//! without the proxy would otherwise fail here while the launcher's own downloads worked.
//!
//! The rules are `launcher/src/proxy.rs`'s, which the launcher applies to its own traffic. The two
//! crates share no code (the launcher depends on this one, not the other way round), so they are
//! kept in step by hand.
use std::sync::Mutex;

/// What the launcher set, when it did.
static PROXY: Mutex<Option<String>> = Mutex::new(None);

/// The proxy to use from now on: the launcher's setting, or None to fall back to the environment.
pub fn set_proxy(spec: Option<String>) {
    let spec = spec.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    if let Ok(mut proxy) = PROXY.lock() {
        *proxy = spec;
    }
}

/// The proxy address to use: the launcher's, else the environment's.
fn spec() -> Option<String> {
    let set = PROXY.lock().ok().and_then(|proxy| proxy.clone());
    set.or_else(|| {
        [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
        ]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        })
    })
}

/// An agent for downloading `url`: through the proxy when one is set, unless `url` is a host that
/// must be reached directly.
pub fn agent(url: &str) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new();
    if direct(url, &no_proxy()) {
        return builder.build();
    }
    match spec().and_then(|s| ureq::Proxy::new(s).ok()) {
        Some(proxy) => builder.proxy(proxy).build(),
        None => builder.build(),
    }
}

/// Whether `url` is reached without the proxy, given a `NO_PROXY` list: a local address, or a host
/// the list names.
fn direct(url: &str, no_proxy: &str) -> bool {
    host_of(url).is_some_and(|host| local(host) || excluded(host, no_proxy))
}

/// `NO_PROXY`'s list: the hosts reached directly whatever the proxy is.
fn no_proxy() -> String {
    ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default()
}

/// The host part of a URL (`https://user@host:port/path` is `host`).
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['/', '?', '#']).next()?;
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    Some(match rest.rfind(':') {
        // A port follows the last colon; an IPv6 literal keeps the colons inside its brackets.
        Some(at) if !rest[at..].contains(']') => &rest[..at],
        _ => rest,
    })
}

/// Whether `host` is this computer.
fn local(host: &str) -> bool {
    let host = bare(host);
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host == "0.0.0.0"
        || host.starts_with("127.")
}

/// Whether `host` is in `NO_PROXY`: `*`, `host`, `.domain`, `*.domain` and `host:port` entries.
fn excluded(host: &str, no_proxy: &str) -> bool {
    let host = bare(host);
    no_proxy
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            let entry = entry.trim_start_matches('*').trim_start_matches('.');
            let entry = match entry.rfind(':') {
                Some(at) if !entry[at..].contains(']') => &entry[..at],
                _ => entry,
            };
            let entry = bare(entry);
            !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
        })
}

/// A host without the brackets an IPv6 literal wears, lower case.
fn bare(host: &str) -> String {
    host.trim().trim_matches(['[', ']']).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_the_launcher_sets_is_trimmed_and_kept() {
        set_proxy(Some("  http://127.0.0.1:7890  ".into()));
        assert_eq!(
            PROXY.lock().unwrap().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        set_proxy(Some("   ".into()));
        assert!(PROXY.lock().unwrap().is_none());
    }

    /// A host that must be reached directly takes no proxy, however one is set: what an import
    /// with `NO_PROXY=modrinth.com` needs, since the launcher's own requests to it go direct.
    #[test]
    fn a_listed_host_is_reached_directly() {
        let list = "modrinth.com";
        assert!(direct("https://cdn.modrinth.com/data/a.jar", list));
        assert!(direct("https://api.modrinth.com/v2/version", list));
        assert!(!direct("https://edge.forgecdn.net/files/1.jar", list));
        assert!(!direct("https://example.com/a.jar", list));
        assert!(direct("http://127.0.0.1:8130/api/v1/status", ""));
    }

    #[test]
    fn a_host_is_read_out_of_the_address() {
        assert_eq!(
            host_of("https://cdn.modrinth.com/v2/x.jar"),
            Some("cdn.modrinth.com")
        );
        assert_eq!(
            host_of("https://user:pass@host.example:8443/a"),
            Some("host.example")
        );
        assert_eq!(host_of("https://[::1]:25565/"), Some("[::1]"));
    }

    #[test]
    fn this_computer_is_never_proxied() {
        for host in ["localhost", "[::1]", "127.0.0.1", "127.1.2.3"] {
            assert!(local(host), "{host}");
        }
        for host in ["cdn.modrinth.com", "10.0.0.1"] {
            assert!(!local(host), "{host}");
        }
    }

    #[test]
    fn no_proxy_lists_hosts_and_their_subdomains() {
        assert!(excluded("anything.at.all", "*"));
        assert!(excluded("cdn.modrinth.com", "modrinth.com"));
        assert!(excluded(
            "wiki.corp.example",
            " .corp.example , other.host "
        ));
        assert!(excluded("github.com", "github.com:443"));
        assert!(!excluded("notmodrinth.com", "modrinth.com"));
        assert!(!excluded("modrinth.com", ""));
    }
}
