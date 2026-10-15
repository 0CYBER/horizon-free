//! Nothing leaves the machine without consent: a local mock server counts every connection the
//! uploader opens, and with the player's consent missing, outdated or withdrawn there are none.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use horizon_telemetry::batch;
use horizon_telemetry::payload::{Bug, Crash, Payload, Perf, Source};
use horizon_telemetry::upload::{self, Flush};
use horizon_telemetry::{Choices, Consent, Item, Kind, outbox};

/// What the mock server saw: one entry a connection, `METHOD path` and the body.
type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// An HTTP server on a free local port answering every request with `status`.
fn mock_server(status: u16) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':')
                    && k.eq_ignore_ascii_case("content-length")
                {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let line = request
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ");
            log.lock().unwrap().push((line, body));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
    });
    (url, seen)
}

fn home(name: &str) -> PathBuf {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).unwrap();
    let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::temp_dir().join(format!("horizon-telemetry-gating-{name}-{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn answer(home: &Path, choices: Choices) -> Consent {
    let mut consent = Consent::load(home);
    consent.answer(choices);
    consent.save(home).unwrap();
    consent
}

fn crash() -> Item {
    Item::new(
        Source::Game,
        "v0.4.2",
        Payload::Crash(Crash {
            error: "panicked at src/game.rs:1:1".into(),
            ..Crash::default()
        }),
    )
}

fn perf() -> Item {
    Item::new(Source::Game, "v0.4.2", Payload::Perf(Perf::default()))
}

/// Puts an item straight into the outbox, as a game from before a withdrawal left it.
fn leftover(home: &Path, item: &Item, name: &str) {
    let dir = outbox::dir(home);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), serde_json::to_vec(item).unwrap()).unwrap();
}

#[test]
fn never_asked_sends_nothing() {
    let (url, seen) = mock_server(204);
    let home = home("never");
    for item in [crash(), perf()] {
        assert!(!outbox::push(&home, &item).unwrap());
    }
    leftover(&home, &crash(), "000001-crash-a.json");
    assert_eq!(upload::flush(&home, &url), Flush::Nothing);
    assert!(seen.lock().unwrap().is_empty());
    assert!(
        outbox::pending(&home).is_empty(),
        "the leftover is deleted, not kept"
    );
}

#[test]
fn everything_off_sends_nothing_and_deletes_what_waited() {
    let (url, seen) = mock_server(204);
    let home = home("withdrawn");
    answer(&home, Choices::ALL);
    assert!(outbox::push(&home, &crash()).unwrap());
    assert!(outbox::push(&home, &perf()).unwrap());
    answer(&home, Choices::NONE);
    assert_eq!(upload::flush(&home, &url), Flush::Nothing);
    assert!(seen.lock().unwrap().is_empty());
    assert!(outbox::pending(&home).is_empty());
}

#[test]
fn an_outdated_consent_sends_nothing() {
    let (url, seen) = mock_server(204);
    let home = home("outdated");
    let mut consent = answer(&home, Choices::ALL);
    assert!(outbox::push(&home, &crash()).unwrap());
    consent.version = 0;
    consent.save(&home).unwrap();
    assert_eq!(upload::flush(&home, &url), Flush::Nothing);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn only_the_allowed_kinds_leave_in_one_compressed_batch() {
    let (url, seen) = mock_server(204);
    let home = home("partial");
    let consent = answer(&home, Choices::ALL);
    for item in [crash(), perf(), crash()] {
        assert!(outbox::push(&home, &item).unwrap());
    }
    answer(
        &home,
        Choices {
            crashes: true,
            ..Choices::NONE
        },
    );
    assert_eq!(
        upload::flush(&home, &url),
        Flush::Sent {
            items: 2,
            batches: 1
        }
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "POST /v1/batch");
    let batch = batch::decode(&seen[0].1).unwrap();
    assert_eq!(batch.install_id, consent.install_id);
    assert!(batch.items.iter().all(|i| i.kind() == Kind::Crash));
    assert_eq!(batch.items.len(), 2);
    assert!(outbox::pending(&home).is_empty());
    drop(seen);
    assert_eq!(upload::flush(&home, &url), Flush::Nothing);
}

#[test]
fn a_failing_server_is_retried_later_and_keeps_the_items() {
    let (url, seen) = mock_server(503);
    let home = home("failing");
    answer(&home, Choices::ALL);
    assert!(outbox::push(&home, &crash()).unwrap());
    assert!(matches!(upload::flush(&home, &url), Flush::Failed(_)));
    assert_eq!(upload::flush(&home, &url), Flush::Later);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "no second try inside the back-off"
    );
    assert_eq!(outbox::pending(&home).len(), 1);
}

#[test]
fn a_refused_batch_is_dropped() {
    let (url, seen) = mock_server(400);
    let home = home("refused");
    answer(&home, Choices::ALL);
    assert!(outbox::push(&home, &crash()).unwrap());
    assert_eq!(upload::flush(&home, &url), Flush::Later);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(outbox::pending(&home).is_empty());
}

#[test]
fn the_preview_is_what_would_be_sent_and_deletion_names_the_install() {
    let (url, seen) = mock_server(204);
    let home = home("preview");
    let consent = answer(
        &home,
        Choices {
            bugs: true,
            ..Choices::NONE
        },
    );
    let bug = Item::new(
        Source::Launcher,
        "0.1.5",
        Payload::Bug(Bug {
            text: "The sky is green".into(),
            ..Bug::default()
        }),
    );
    assert!(outbox::push(&home, &bug).unwrap());
    leftover(&home, &crash(), "000001-crash-a.json");
    let preview: serde_json::Value = serde_json::from_str(&upload::preview(&home)).unwrap();
    assert_eq!(preview["items"].as_array().unwrap().len(), 1);
    assert_eq!(preview["items"][0]["data"]["text"], "The sky is green");
    assert!(seen.lock().unwrap().is_empty(), "previewing sends nothing");
    upload::delete_remote(&url, &consent.install_id).unwrap();
    assert!(upload::delete_remote(&url, "").is_err());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].0,
        format!("DELETE /v1/installs/{}", consent.install_id)
    );
}
