//! The proxy every download and request goes through, when the player needs one: a school's or an
//! office's, or a country where a host is unreachable. The Settings page's Proxy row (kept as
//! `proxy` in `settings.json`) wins; without it the usual `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`
//! and `NO_PROXY` variables apply, so a launcher started from a shell that has them needs nothing.
//!
//! What goes through it: Horizon's releases and the launcher's own update, Minecraft's files and
//! Java from Mojang, Modrinth, CurseForge, Horizon's mod registry, pictures, Microsoft sign-in,
//! Tailscale's API, and the jars a modpack import downloads. A local address never does, and the
//! Servers page's calls to a game server's API (`servers.rs`) build their agent without it: they
//! are on loopback and asked every second.
use crate::backend::Settings;

/// The proxy to reach `url` through: the one set for the launcher or in the environment, unless
/// `url` is a local address or `NO_PROXY` names its host.
pub fn for_url(url: &str) -> Option<ureq::Proxy> {
    let scheme = scheme_of(url)?;
    let host = host_of(url)?;
    if local(host) || excluded(host, &no_proxy()) {
        return None;
    }
    ureq::Proxy::new(written(scheme)?).ok()
}

/// `builder`, reaching `url` through the proxy when one is set.
pub fn agent(builder: ureq::AgentBuilder, url: &str) -> ureq::AgentBuilder {
    match for_url(url) {
        Some(proxy) => builder.proxy(proxy),
        None => builder,
    }
}

/// Why `spec` can't be used as a proxy, or None when it can (empty means none).
pub fn refuse(spec: &str) -> Option<String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let scheme = spec.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    if matches!(
        scheme.as_deref(),
        Some("socks" | "socks4" | "socks4a" | "socks5" | "socks5h")
    ) {
        return Some(format!(
            "{spec} is a SOCKS proxy; Horizon downloads through an HTTP proxy, such as the \
             http:// address your proxy app also listens on."
        ));
    }
    // `Proxy::new` takes `http://` without a host, which would leave a download failing with
    // nothing said about the proxy.
    if ureq::Proxy::new(spec).is_err() || host_of(spec).is_none_or(str::is_empty) {
        return Some(format!(
            "{spec} isn't a proxy address. Write it as http://host:port."
        ));
    }
    None
}

/// Tells the parts that build their own agents (`horizon-port-pack`) what to use. Called at start
/// and whenever the player changes the setting.
pub fn announce() {
    horizon_port_pack::net::set_proxy(written("https"));
}

/// The proxy as it is written down: the launcher's, else the environment's for `scheme`.
fn written(scheme: &str) -> Option<String> {
    Settings::load()
        .proxy
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .or_else(|| from_env(scheme))
}

/// `HTTPS_PROXY` (`HTTP_PROXY` for an http address), then `ALL_PROXY`, in the upper-case spelling
/// first and the lower-case one after, as the tools that write them do.
fn from_env(scheme: &str) -> Option<String> {
    let names: &[&str] = if scheme.eq_ignore_ascii_case("http") {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    };
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
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

/// `https` in `https://…`, or None for anything that isn't an absolute address.
fn scheme_of(url: &str) -> Option<&str> {
    url.split_once("://").map(|(scheme, _)| scheme)
}

/// Whether `host` is this computer: the game's own API and a server on the same machine.
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
    fn a_host_is_read_out_of_the_address() {
        assert_eq!(
            host_of("https://cdn.modrinth.com/v2/x.jar"),
            Some("cdn.modrinth.com")
        );
        assert_eq!(
            host_of("https://user:pass@piston-meta.mojang.com:8443/a"),
            Some("piston-meta.mojang.com")
        );
        assert_eq!(
            host_of("http://127.0.0.1:8130/api/v1/status"),
            Some("127.0.0.1")
        );
        assert_eq!(host_of("https://[::1]:25565/"), Some("[::1]"));
        assert_eq!(host_of("https://example.com?x=1"), Some("example.com"));
        assert_eq!(host_of("https://example.com#top"), Some("example.com"));
    }

    #[test]
    fn only_absolute_addresses_have_a_scheme() {
        assert_eq!(scheme_of("https://a.b"), Some("https"));
        assert_eq!(scheme_of("http://a.b"), Some("http"));
        assert_eq!(scheme_of("a.b/c"), None);
    }

    #[test]
    fn this_computer_is_never_proxied() {
        for host in [
            "localhost",
            "LOCALHOST",
            "[::1]",
            "::1",
            "127.0.0.1",
            "127.1.2.3",
        ] {
            assert!(local(host), "{host}");
        }
        for host in ["playhorizon.dev", "api.minecraftservices.com", "10.0.0.1"] {
            assert!(!local(host), "{host}");
        }
    }

    #[test]
    fn no_proxy_lists_hosts_and_their_subdomains() {
        let list = "*.internal, .corp.example, mojang.com, github.com:443, *";
        assert!(excluded("anything.at.all", list));
        assert!(excluded("wiki.corp.example", "corp.example"));
        assert!(excluded("piston-meta.mojang.com", "mojang.com"));
        assert!(excluded("github.com", "github.com:443"));
        assert!(!excluded("notmojang.com", "mojang.com"));
        assert!(!excluded("modrinth.com", " .corp.example , mojang.com "));
        assert!(!excluded("modrinth.com", ""));
    }

    #[test]
    fn a_proxy_address_is_checked_before_it_is_kept() {
        assert_eq!(refuse(""), None);
        assert_eq!(refuse("  "), None);
        assert_eq!(refuse("http://127.0.0.1:7890"), None);
        assert_eq!(refuse("127.0.0.1:7890"), None);
        assert_eq!(refuse("http://user:pass@proxy.local:3128"), None);
        assert!(refuse("http://").is_some());
        assert!(refuse("socks5://127.0.0.1:7891").is_some());
    }
}
