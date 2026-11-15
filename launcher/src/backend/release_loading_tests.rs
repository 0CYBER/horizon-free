use super::*;
use std::cell::RefCell;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn fixture() -> String {
    let mut list: Vec<_> = (1..=99)
        .map(|i| {
            serde_json::json!({
                "tag_name": format!("v2026.10.14.{i}"),
                "name": format!("Build {i}"), "body": "Release notes\r\n".repeat(256),
                "published_at": "2026-10-14T12:00:00Z", "draft": i == 99,
                "assets": [{"name": ASSET.unwrap_or("unsupported"),
                    "browser_download_url": "https://example.test/game", "size": 12345}]
            })
        })
        .collect();
    list.push(serde_json::json!({
        "tag_name": "launcher@99.0.0", "name": "Launcher", "body": "Notes",
        "assets": [{"name": LAUNCHER_ASSET.unwrap_or("unsupported"),
            "browser_download_url": "https://example.test/launcher", "size": 6789}]
    }));
    serde_json::to_string(&list).unwrap()
}

fn installed_fixture() -> Vec<String> {
    vec!["v2026.10.14.50".into()]
}

// The former startup paths: each decodes its own response; the update path parses games again.
fn baseline(load: impl Fn() -> Result<serde_json::Value, String>) -> (Vec<Release>, Updates) {
    let releases = parse_releases(&load().unwrap(), ASSET, is_game_tag);
    let list = load().unwrap();
    let updates = Updates {
        horizon: horizon_update(
            &parse_releases(&list, ASSET, is_game_tag),
            &installed_fixture(),
        ),
        launcher: launcher_update(&parse_releases(&list, LAUNCHER_ASSET, is_launcher_tag)),
    };
    (releases, updates)
}

fn shared(load: impl FnOnce() -> Result<serde_json::Value, String>) -> (Vec<Release>, Updates) {
    let mut releases = None;
    let updates = releases_and_updates_with(
        load,
        installed_fixture,
        |list| parse_releases(list, LAUNCHER_ASSET, is_launcher_tag),
        |r| releases = Some(r.unwrap()),
    )
    .unwrap();
    (releases.unwrap(), updates)
}

#[test]
fn one_response_preserves_both_startup_results() {
    let text = fixture();
    let requests = AtomicUsize::new(0);
    let load = || {
        requests.fetch_add(1, Ordering::Relaxed);
        Ok(serde_json::from_str(&text).unwrap())
    };
    let old = baseline(load);
    assert_eq!(requests.swap(0, Ordering::Relaxed), 2);
    let new = shared(load);
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    assert_eq!(
        serde_json::to_value(old).unwrap(),
        serde_json::to_value(new).unwrap()
    );
}

#[test]
fn errors_reach_releases_and_updates_without_fallback() {
    for error in [
        "No connection: offline",
        "GitHub's rate limit was reached.",
        "invalid JSON",
    ] {
        let mut published = None;
        let result = releases_and_updates_with(
            || Err(error.into()),
            || panic!("no installed scan after failure"),
            |_| panic!("no fallback after failure"),
            |r| published = Some(r),
        );
        assert_eq!(published.unwrap().unwrap_err(), error);
        assert_eq!(result.unwrap_err(), error);
    }
}

#[test]
fn publish_games_before_older_launcher_fallback() {
    let events = RefCell::new(Vec::new());
    let result = releases_and_updates_with(
        || Ok(serde_json::json!([{"tag_name": "v2026.10.14", "assets": []}])),
        Vec::new,
        |list| {
            launcher_releases_in_with(list, |url| {
                assert_eq!(events.borrow().first(), Some(&"releases"));
                events.borrow_mut().push("fallback");
                if url.contains("matching-refs") {
                    Ok(Some(
                        serde_json::json!([{"ref": "refs/tags/launcher@99.0.0"}]),
                    ))
                } else {
                    Ok(Some(
                        serde_json::json!({"tag_name": "launcher@99.0.0", "assets": [{
                            "name": LAUNCHER_ASSET.unwrap_or("unsupported"),
                            "browser_download_url": "u", "size": 1
                        }]}),
                    ))
                }
            })
        },
        |r| {
            assert_eq!(r.unwrap()[0].tag, "v2026.10.14");
            events.borrow_mut().push("releases");
        },
    )
    .unwrap();
    assert_eq!(*events.borrow(), ["releases", "fallback", "fallback"]);
    assert_eq!(result.launcher.is_some(), LAUNCHER_ASSET.is_some());
}

#[test]
fn failed_launcher_fallback_keeps_game_result_and_checks_stay_fresh() {
    let requests = AtomicUsize::new(0);
    for day in [14, 15] {
        let mut published = None;
        let updates = releases_and_updates_with(
            || {
                requests.fetch_add(1, Ordering::Relaxed);
                Ok(serde_json::json!([{"tag_name": format!("v2026.10.{day}")}]))
            },
            Vec::new,
            |list| launcher_releases_in_with(list, |_| Err("offline".into())),
            |r| published = Some(r.unwrap()),
        )
        .unwrap();
        assert_eq!(published.unwrap()[0].tag, format!("v2026.10.{day}"));
        assert!(updates.launcher.is_none());
    }
    assert_eq!(requests.load(Ordering::Relaxed), 2);
}

#[test]
fn empty_or_prebuilt_only_fork_has_no_game_or_launcher_update() {
    for list in [
        serde_json::json!([]),
        serde_json::json!([{
            "tag_name": "prebuilt-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "prerelease": true,
            "assets": [{"name": "horizon-prebuilt-v3-x86_64-unknown-linux-gnu.tar.gz",
                        "browser_download_url": "https://example.test/sdk", "size": 9}]
        }]),
    ] {
        for missing in [None, Some(serde_json::json!([]))] {
            let mut games = None;
            let updates = releases_and_updates_with(
                || Ok(list.clone()),
                Vec::new,
                |list| {
                    launcher_releases_in_with(list, |url| {
                        assert!(url.starts_with("https://api.github.com/repos/arismoko/horizon/"));
                        Ok(missing.clone())
                    })
                },
                |result| games = Some(result.unwrap()),
            )
            .unwrap();
            assert!(games.unwrap().is_empty());
            assert!(updates.horizon.is_none());
            assert!(updates.launcher.is_none());
        }
    }
}

#[test]
fn each_platform_download_is_bound_to_the_fork_not_the_provider_url() {
    let platforms = [
        (
            "linux",
            "x86_64",
            "horizon-linux-x86_64.tar.gz",
            "horizon-launcher-linux-x86_64.tar.gz",
        ),
        (
            "windows",
            "x86_64",
            "horizon-windows-x64.zip",
            "horizon-launcher-windows-x64.zip",
        ),
        (
            "macos",
            "aarch64",
            "horizon-macos-arm64.zip",
            "horizon-launcher-macos-arm64.zip",
        ),
    ];
    let assets: Vec<_> = platforms.iter().flat_map(|(_, _, game, launcher)| [game, launcher])
        .map(|name| serde_json::json!({"name": name, "size": 9,
            "browser_download_url": "https://github.com/horizon-rust/horizon/releases/download/wrong/file"}))
        .collect();
    let list = serde_json::json!([
        {"tag_name": "v0.9.9", "assets": assets},
        {"tag_name": "launcher@0.9.9", "assets": assets}
    ]);
    for (os, arch, game, launcher) in platforms {
        let (game_asset, launcher_asset) = platform_assets(os, arch).unwrap();
        for (asset, keep, expected, tag) in [
            (game_asset, is_game_tag as fn(&str) -> bool, game, "v0.9.9"),
            (
                launcher_asset,
                is_launcher_tag as fn(&str) -> bool,
                launcher,
                "launcher@0.9.9",
            ),
        ] {
            let parsed = parse_releases(&list, Some(asset), keep);
            assert_eq!(parsed.len(), 1);
            assert_eq!(
                parsed[0].asset,
                Some((
                    format!(
                        "https://github.com/arismoko/horizon/releases/download/{tag}/{expected}"
                    ),
                    9
                ))
            );
        }
    }
}

#[test]
fn decoy_asset_names_never_replace_the_exact_platform_asset() {
    let asset = "horizon-linux-x86_64.tar.gz";
    for decoy in [
        format!("../{asset}"),
        format!("prefix-{asset}"),
        format!("{asset}.bak"),
    ] {
        for (tag, encoded) in [
            ("v1/../../x", "v1%2F..%2F..%2Fx"),
            ("v1%?# é", "v1%25%3F%23%20%C3%A9"),
        ] {
            let list = serde_json::json!([{ "tag_name": tag, "html_url": "https://evil.test", "assets": [
                {"name": decoy, "size": 1, "browser_download_url": "https://evil.test/first"},
                {"name": asset, "size": 2, "browser_download_url": "https://evil.test/second"}
            ]}]);
            let got = parse_releases(&list, Some(asset), is_game_tag);
            assert_eq!(
                got[0].asset,
                Some((
                    format!(
                        "https://github.com/arismoko/horizon/releases/download/{encoded}/{asset}"
                    ),
                    2
                ))
            );
        }
    }
    for tag in ["../x", "prebuilt-1"] {
        let got = game_releases_with(
            &serde_json::json!([{"tag_name": tag, "prerelease": true, "assets": [{"name": asset, "size": 1}]}]),
            Some(asset),
            None,
        );
        assert!(got.is_empty());
    }
    let draft = parse_releases(
        &serde_json::json!([{"tag_name": "v1", "draft": true, "assets": [{"name": asset, "size": 1}]}]),
        Some(asset),
        is_game_tag,
    );
    assert!(draft.is_empty());
    assert_eq!(
        crate::signed::asset_url("../../x", "../evil.zip"),
        "https://github.com/arismoko/horizon/releases/download/..%2F..%2Fx/..%2Fevil.zip"
    );
}

#[test]
#[ignore = "local release-loading benchmark; run explicitly with --ignored --nocapture"]
fn benchmark_shared_startup_response() {
    let text = fixture();
    let requests = AtomicUsize::new(0);
    let load = |delay: Duration| {
        requests.fetch_add(1, Ordering::Relaxed);
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        serde_json::from_str(black_box(&text)).map_err(|e| e.to_string())
    };
    let old = || baseline(|| load(Duration::ZERO));
    let new = || shared(|| load(Duration::ZERO));
    assert_eq!(
        serde_json::to_value(old()).unwrap(),
        serde_json::to_value(new()).unwrap()
    );
    let concurrent_old = || {
        std::thread::scope(|scope| {
            let releases = scope.spawn(|| {
                parse_releases(&load(Duration::from_millis(5)).unwrap(), ASSET, is_game_tag)
            });
            let updates = scope.spawn(|| {
                let list = load(Duration::from_millis(5)).unwrap();
                Updates {
                    horizon: horizon_update(
                        &parse_releases(&list, ASSET, is_game_tag),
                        &installed_fixture(),
                    ),
                    launcher: launcher_update(&parse_releases(
                        &list,
                        LAUNCHER_ASSET,
                        is_launcher_tag,
                    )),
                }
            });
            (releases.join().unwrap(), updates.join().unwrap())
        })
    };
    let threaded_new = || {
        std::thread::scope(|scope| {
            scope
                .spawn(|| shared(|| load(Duration::from_millis(5))))
                .join()
                .unwrap()
        })
    };
    assert_eq!(
        serde_json::to_value(concurrent_old()).unwrap(),
        serde_json::to_value(threaded_new()).unwrap()
    );
    fn measure<T>(run: impl Fn() -> T, requests: &AtomicUsize) -> (u128, usize) {
        for _ in 0..20 {
            black_box(run());
        }
        requests.store(0, Ordering::Relaxed);
        let mut times: Vec<_> = (0..201)
            .map(|_| {
                let start = Instant::now();
                black_box(run());
                start.elapsed().as_nanos()
            })
            .collect();
        times.sort_unstable();
        (times[100], requests.load(Ordering::Relaxed))
    }
    println!(
        "fixture={} bytes; 100 releases; 20 warmups, 201 samples; median ns",
        text.len()
    );
    for (label, before, after) in [
        (
            "serial zero-delay work",
            measure(old, &requests),
            measure(new, &requests),
        ),
        (
            "concurrent startup, 5ms/request",
            measure(concurrent_old, &requests),
            measure(threaded_new, &requests),
        ),
    ] {
        assert_eq!((before.1, after.1), (402, 201));
        println!(
            "{label}: before={} after={} ratio={:.3}x requests={}->{}",
            before.0,
            after.0,
            before.0 as f64 / after.0 as f64,
            before.1,
            after.1
        );
    }
}
