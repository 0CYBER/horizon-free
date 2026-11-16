use super::*;
use std::cell::RefCell;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-github-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

const T0: u64 = 1_800_000_000;
const LATER: u64 = T0 + FRESH_SECS;

const RELEASES: &str = "https://api.github.com/repos/horizon-rust/horizon/releases?per_page=100";
const FORK_RELEASES: &str = "https://api.github.com/repos/arismoko/horizon/releases?per_page=100";

fn list(tag: &str) -> serde_json::Value {
    serde_json::json!([{ "tag_name": tag }])
}

fn no_mirror(_: &str) -> Answer {
    panic!("the mirror is only asked when GitHub can't answer")
}

#[test]
fn mirror_serves_only_horizons_repository() {
    let base = "https://mcp.example/github/public";
    assert_eq!(
        mirror_url_with(RELEASES, base).as_deref(),
        Some("https://mcp.example/github/public/repos/horizon-rust/horizon/releases?per_page=100")
    );
    assert_eq!(
        mirror_url_with(
            "https://api.github.com/repos/horizon-rust/horizon/releases/tags/launcher@0.2.33",
            &format!("{base}/")
        )
        .as_deref(),
        Some("https://mcp.example/github/public/repos/horizon-rust/horizon/releases/tags/launcher@0.2.33")
    );
    assert_eq!(
        mirror_url_with("https://api.github.com/repos/someone/mod/commits/abc", base),
        None
    );
    assert_eq!(
        mirror_url_with("https://github.com/horizon-rust/horizon/releases", base),
        None
    );
    for url in [
        "https://api.github.com/repos/arismoko/horizon/releases",
        "https://api.github.com/repos/horizon-rust/horizon-extra/releases",
    ] {
        assert_eq!(mirror_url_with(url, base), None);
    }
    assert_eq!(
        mirror_url_with(RELEASES, ""),
        None,
        "an empty mirror is off"
    );
}

#[test]
fn a_saved_upstream_list_cannot_answer_a_fork_check() {
    let cache = scratch("repository-scope");
    json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::Fresh(list("upstream"), None),
        no_mirror,
    )
    .unwrap();
    let absent = json_with(
        FORK_RELEASES,
        &cache,
        T0,
        |_, _| Answer::NotFound,
        no_mirror,
    )
    .unwrap();
    assert_eq!(absent, None);
    let error = json_with(
        FORK_RELEASES,
        &cache,
        T0,
        |_, _| Answer::Failed("offline".into()),
        |_| Answer::Failed("offline".into()),
    )
    .unwrap_err();
    assert!(error.contains("offline"));
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn rate_limit_is_429_or_403_with_githubs_headers() {
    let none = |_: &str| None;
    assert!(rate_limited(429, none));
    assert!(!rate_limited(403, none), "a plain 403 is a refusal");
    assert!(rate_limited(403, |h| (h == "x-ratelimit-remaining").then(|| "0".into())));
    assert!(!rate_limited(403, |h| (h == "x-ratelimit-remaining").then(|| "12".into())));
    assert!(rate_limited(403, |h| (h == "retry-after").then(|| "60".into())));
    assert!(!rate_limited(500, |h| (h == "retry-after").then(|| "60".into())));
}

#[test]
fn etag_is_kept_and_a_304_reads_the_saved_copy() {
    let cache = scratch("etag");
    let sent = RefCell::new(Vec::new());
    let first = json_with(
        RELEASES,
        &cache,
        T0,
        |_, etag| {
            sent.borrow_mut().push(etag.map(str::to_owned));
            Answer::Fresh(list("nightly"), Some("\"abc\"".into()))
        },
        no_mirror,
    );
    assert_eq!(first.unwrap(), Some(list("nightly")));
    // Started again within FRESH_SECS: the saved list, without asking.
    let again = json_with(
        RELEASES,
        &cache,
        T0 + FRESH_SECS - 1,
        |_, _| panic!("a fresh copy is used without asking"),
        no_mirror,
    );
    assert_eq!(again.unwrap(), Some(list("nightly")));
    // Later: a conditional request; a 304 gives the saved list and starts a new fresh window.
    let second = json_with(
        RELEASES,
        &cache,
        LATER,
        |_, etag| {
            sent.borrow_mut().push(etag.map(str::to_owned));
            Answer::NotModified
        },
        no_mirror,
    );
    assert_eq!(second.unwrap(), Some(list("nightly")));
    assert_eq!(*sent.borrow(), [None, Some("\"abc\"".to_owned())]);
    let third = json_with(
        RELEASES,
        &cache,
        LATER + FRESH_SECS - 1,
        |_, _| panic!("confirmed by the 304"),
        no_mirror,
    );
    assert_eq!(third.unwrap(), Some(list("nightly")));
    // A clock set back doesn't make the copy fresh forever.
    let back = json_with(
        RELEASES,
        &cache,
        T0 - 1,
        |_, _| Answer::Fresh(list("v9"), None),
        no_mirror,
    );
    assert_eq!(back.unwrap(), Some(list("v9")));
    // Another URL doesn't read this one's copy.
    let other = "https://api.github.com/repos/horizon-rust/horizon/releases/latest";
    let etag_sent = RefCell::new(Some(String::new()));
    let _ = json_with(
        other,
        &cache,
        T0,
        |_, etag| {
            *etag_sent.borrow_mut() = etag.map(str::to_owned);
            Answer::Fresh(list("v1"), None)
        },
        no_mirror,
    );
    assert_eq!(*etag_sent.borrow(), None);
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn rate_limited_reads_go_through_the_mirror() {
    let cache = scratch("mirror");
    let asked = RefCell::new(None);
    let got = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        |url| {
            *asked.borrow_mut() = Some(url.to_owned());
            Answer::Fresh(list("nightly"), None)
        },
    );
    assert_eq!(got.unwrap(), Some(list("nightly")));
    assert!(asked
        .borrow()
        .as_deref()
        .unwrap()
        .ends_with("/github/public/repos/horizon-rust/horizon/releases?per_page=100"));
    // The mirror's answer is kept like GitHub's.
    let kept = json_with(RELEASES, &cache, T0 + 1, |_, _| panic!("fresh"), no_mirror);
    assert_eq!(kept.unwrap(), Some(list("nightly")));
    // No connection to GitHub (blocked, DNS): the mirror too.
    let got = json_with(
        RELEASES,
        &cache,
        LATER,
        |_, _| Answer::Failed("No connection: dns".into()),
        |_| Answer::Fresh(list("v2"), None),
    );
    assert_eq!(got.unwrap(), Some(list("v2")));
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn a_404_is_not_retried_on_the_mirror() {
    let cache = scratch("404");
    let latest = "https://api.github.com/repos/horizon-rust/horizon/releases/latest";
    let absent = json_with(
        latest,
        &cache,
        T0,
        |_, _| {
            failure(
                latest,
                ureq::Error::Status(404, ureq::Response::new(404, "Not Found", "{}").unwrap()),
            )
        },
        no_mirror,
    )
    .unwrap();
    assert_eq!(absent, None);
    let absent = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        |_| Answer::NotFound,
    )
    .unwrap();
    assert_eq!(absent, None);
    let empty = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::Fresh(serde_json::json!([]), None),
        no_mirror,
    )
    .unwrap();
    assert_eq!(empty, Some(serde_json::json!([])));
    // A failure whose text mentions 404 is not an absent release.
    let url = format!("{latest}?note=404");
    let error = json_with(
        &url,
        &cache,
        T0,
        |_, _| {
            failure(
                &url,
                ureq::Error::Status(
                    500,
                    ureq::Response::new(500, "Server Error", "404 in the body").unwrap(),
                ),
            )
        },
        |_| Answer::Failed("offline".into()),
    )
    .unwrap_err();
    assert!(error.contains("answered 500"), "{error}");
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn both_failing_uses_the_saved_copy_else_says_so() {
    let cache = scratch("both");
    let err = json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        |_| Answer::Failed("No connection: timed out".into()),
    )
    .unwrap_err();
    assert!(err.contains("rate limit"), "{err}");
    assert!(err.contains("mirror"), "{err}");
    assert!(err.contains("timed out"), "{err}");
    assert!(err.contains("Try again in a few minutes"), "{err}");

    // With a copy from an earlier start, that copy is better than nothing.
    json_with(
        RELEASES,
        &cache,
        T0,
        |_, _| Answer::Fresh(list("nightly"), Some("\"e\"".into())),
        no_mirror,
    )
    .unwrap();
    let got = json_with(
        RELEASES,
        &cache,
        LATER,
        |_, _| Answer::RateLimited,
        |_| Answer::RateLimited,
    );
    assert_eq!(got.unwrap(), Some(list("nightly")));
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn other_repositories_have_no_mirror() {
    let cache = scratch("other");
    let err = json_with(
        "https://api.github.com/repos/someone/mod/commits/abc",
        &cache,
        T0,
        |_, _| Answer::RateLimited,
        no_mirror,
    )
    .unwrap_err();
    assert!(err.contains("rate limit"), "{err}");
    assert!(!err.contains("mirror"), "{err}");
    let _ = fs::remove_dir_all(cache);
}

#[test]
fn a_corrupt_or_foreign_copy_is_ignored() {
    let cache = scratch("corrupt");
    fs::write(cache.join(cache_name(RELEASES)), b"not json").unwrap();
    let got = json_with(
        RELEASES,
        &cache,
        T0,
        |_, etag| {
            assert_eq!(etag, None);
            Answer::Fresh(list("v1"), None)
        },
        no_mirror,
    );
    assert_eq!(got.unwrap(), Some(list("v1")));
    let foreign = Kept {
        url: "https://api.github.com/other".into(),
        etag: "\"x\"".into(),
        saved: T0,
        body: list("x"),
    };
    fs::write(
        cache.join(cache_name(RELEASES)),
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    let got = json_with(
        RELEASES,
        &cache,
        T0,
        |_, etag| {
            assert_eq!(etag, None);
            Answer::Fresh(list("v2"), None)
        },
        no_mirror,
    );
    assert_eq!(got.unwrap(), Some(list("v2")));
    let _ = fs::remove_dir_all(cache);
}
