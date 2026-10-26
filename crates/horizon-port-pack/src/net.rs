//! The agents port-pack's downloads go through, since the jars of an imported modpack come from
//! the same places as everything else and may need the same proxy.
//!
//! The launcher hands its own setting over (`set_proxy`, from `proxy.rs`); a run of the
//! `horizon-port-pack` command reads `HTTPS_PROXY`, `HTTP_PROXY` and `ALL_PROXY` instead. A
//! `NO_PROXY` list is the launcher's business: it decides per address, and nothing here is local.
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

/// An agent for a download, through the proxy when one is set.
pub fn agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new();
    if let Some(proxy) = spec().and_then(|s| ureq::Proxy::new(s).ok()) {
        builder = builder.proxy(proxy);
    }
    builder.build()
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
}
