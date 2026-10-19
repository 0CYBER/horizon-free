use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

fn running() -> Value {
    json!({
        "BackendState": "Running",
        "AuthURL": "",
        "Self": {
            "ID": "nAbCdEf1CNTRL",
            "HostName": "ludos-mac",
            "DNSName": "ludos-mac.tail1234.ts.net.",
            "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0::1"]
        },
        "CurrentTailnet": { "Name": "ludo@example.com", "MagicDNSSuffix": "tail1234.ts.net", "MagicDNSEnabled": true }
    })
}

#[test]
fn status_reads_as_the_cli_writes_it() {
    let State::Running(node) = parse_status(&running()) else {
        panic!("not running");
    };
    assert_eq!(node.id, "nAbCdEf1CNTRL");
    assert_eq!(node.dns_name, "ludos-mac.tail1234.ts.net");
    assert_eq!(node.ipv4(), Some("100.101.102.103"));
    assert_eq!(node.address(25565), "ludos-mac.tail1234.ts.net");
    assert_eq!(
        Node {
            magic_dns: false,
            ..node.clone()
        }
        .address(25570),
        "100.101.102.103:25570"
    );
    assert_eq!(
        parse_status(
            &json!({ "BackendState": "NeedsLogin", "AuthURL": "https://login.tailscale.com/a/abc" })
        ),
        State::SignedOut {
            auth_url: Some("https://login.tailscale.com/a/abc".into())
        }
    );
    assert_eq!(
        parse_status(&json!({ "BackendState": "NeedsLogin", "AuthURL": "" })),
        State::SignedOut { auth_url: None }
    );
    assert!(matches!(
        parse_status(&json!({ "BackendState": "Stopped" })),
        State::Off(_)
    ));
    assert_eq!(status_of(None), State::Missing);
}

/// The CLI is only ever asked `status --json`: a stand-in that refuses anything else.
#[cfg(unix)]
#[test]
fn status_asks_the_cli_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("horizon-tailscale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let json = dir.join("status.json");
    std::fs::write(&json, running().to_string()).unwrap();
    let fake = dir.join("tailscale");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\n[ \"$*\" = \"status --json\" ] || {{ echo \"unexpected: $*\" >&2; exit 9; }}\ncat '{}'\n",
            json.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(status_of(Some(fake.clone())), State::Running(n) if n.host == "ludos-mac"));
    // A stopped daemon: no JSON, a line on stderr.
    std::fs::write(
        &fake,
        "#!/bin/sh\necho 'failed to connect to local tailscaled; it doesn'\\''t appear to be running' >&2\nexit 1\n",
    )
    .unwrap();
    assert!(matches!(status_of(Some(fake)), State::Off(why) if why.contains("tailscaled")));
    std::fs::remove_dir_all(&dir).unwrap();
}

fn default_acls() -> Value {
    json!({
        "tagOwners": { "tag:server": ["autogroup:admin"] },
        "acls": [{ "action": "accept", "src": ["*"], "dst": ["*:*"] }],
        "ssh": [{ "action": "check", "src": ["autogroup:member"], "dst": ["autogroup:self"], "users": ["autogroup:nonroot", "root"] }]
    })
}

#[test]
fn the_default_policy_is_limited_for_shared_friends_only() {
    let ip = "100.101.102.103";
    let policy = default_acls();
    assert_eq!(reach(&policy, ip, 25565), Reach::Everything);
    let limited = limit_shared(&policy, ip, 25565).unwrap();
    assert_eq!(
        limited["acls"],
        json!([
            { "action": "accept", "src": ["autogroup:member", "autogroup:tagged"], "dst": ["*:*"] },
            { "action": "accept", "src": ["autogroup:shared"], "dst": ["100.101.102.103:25565"] }
        ])
    );
    // Everything else stays.
    assert_eq!(limited["ssh"], policy["ssh"]);
    assert_eq!(limited["tagOwners"], policy["tagOwners"]);
    assert_eq!(reach(&limited, ip, 25565), Reach::ServerOnly);
    // Another port: not the launcher's rule for this server.
    assert_eq!(reach(&limited, ip, 25566), Reach::Custom);
    assert!(limit_shared(&limited, ip, 25565).is_none());
    // Undone, it's Tailscale's default again.
    let undone = unlimit_shared(&limited, ip, 25565).unwrap();
    assert_eq!(reach(&undone, ip, 25565), Reach::Everything);
    assert_eq!(undone["ssh"], policy["ssh"]);

    // The older form of the default rule.
    let old = json!({ "acls": [{ "action": "accept", "users": ["*"], "ports": ["*:*"] }] });
    assert_eq!(reach(&old, ip, 25565), Reach::Everything);
    assert!(limit_shared(&old, ip, 25565).is_some());

    // Grants.
    let grants = json!({ "grants": [{ "src": ["*"], "dst": ["*"], "ip": ["*"] }] });
    let limited = limit_shared(&grants, ip, 25570).unwrap();
    assert_eq!(
        limited["grants"][1],
        json!({ "src": ["autogroup:shared"], "dst": [ip], "ip": ["tcp:25570"] })
    );
    assert_eq!(reach(&limited, ip, 25570), Reach::ServerOnly);
    assert!(unlimit_shared(&limited, ip, 25570).is_some());

    // Someone's own rules: never rewritten.
    let custom = json!({ "acls": [
        { "action": "accept", "src": ["group:family"], "dst": ["*:*"] },
        { "action": "accept", "src": ["*"], "dst": ["*:*"] }
    ] });
    assert_eq!(reach(&custom, ip, 25565), Reach::Custom);
    assert!(limit_shared(&custom, ip, 25565).is_none());
    assert!(unlimit_shared(&custom, ip, 25565).is_none());
    let both = json!({ "acls": [{ "action": "accept", "src": ["*"], "dst": ["*:*"] }], "grants": [{ "src": ["*"], "dst": ["*"], "ip": ["*"] }] });
    assert_eq!(reach(&both, ip, 25565), Reach::Custom);
    assert!(snippet(ip, 25565).contains("\"dst\": [\"100.101.102.103:25565\"]"));
}

/// What the stand-in API saw: request line, headers, body.
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

/// A stand-in for api.tailscale.com answering each request with the next reply (status, ETag, body).
fn stand_in(replies: Vec<(u16, Option<&'static str>, String)>) -> (String, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for (code, etag, body) in replies {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            let length = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut sent = vec![0; length];
            reader.read_exact(&mut sent).unwrap();
            log.lock().unwrap().push((
                first.trim().to_string(),
                head,
                String::from_utf8(sent).unwrap(),
            ));
            let reason = match code {
                200 => "OK",
                401 => "Unauthorized",
                _ => "Error",
            };
            let etag = etag.map(|e| format!("ETag: {e}\r\n")).unwrap_or_default();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (base, seen)
}

#[test]
fn invites_and_access_controls_go_through_the_api() {
    let invite = json!([{
        "id": "12345", "created": "2026-10-19T12:00:00Z", "inviteUrl": "https://login.tailscale.com/admin/invite/code1",
        "accepted": false
    }]);
    let accepted = json!([{
        "id": 12346, "created": "2026-10-18T12:00:00Z", "inviteUrl": "https://login.tailscale.com/admin/invite/code2",
        "accepted": true, "acceptedBy": { "id": 1, "loginName": "friend@example.com" }
    }]);
    let (base, seen) = stand_in(vec![
        (200, None, invite.to_string()),
        (200, None, accepted.to_string()),
        (200, None, String::new()),
        (200, Some("\"e1\""), default_acls().to_string()),
        (200, None, "{}".into()),
        (200, None, "{}".into()),
        (
            401,
            None,
            json!({ "message": "API token invalid" }).to_string(),
        ),
    ]);
    let api = Api::at(&base, " tskey-api-test\n");
    let made = api.invite("nAbC", None).unwrap();
    assert_eq!(made.url, "https://login.tailscale.com/admin/invite/code1");
    assert!(!made.accepted);
    let list = api.invites("nAbC").unwrap();
    assert_eq!(list[0].id, "12346");
    assert_eq!(list[0].accepted_by.as_deref(), Some("friend@example.com"));
    api.revoke("12346").unwrap();
    let (policy, etag) = api.policy().unwrap();
    assert_eq!(etag.as_deref(), Some("\"e1\""));
    let limited = limit_shared(&policy, "100.101.102.103", 25565).unwrap();
    api.validate(&limited).unwrap();
    api.set_policy(&limited, etag.as_deref()).unwrap();
    assert!(api.invites("nAbC").unwrap_err().contains("access token"));

    let seen = seen.lock().unwrap();
    let lines: Vec<&str> = seen.iter().map(|(l, _, _)| l.as_str()).collect();
    assert_eq!(
        lines,
        [
            "POST /api/v2/device/nAbC/device-invites HTTP/1.1",
            "GET /api/v2/device/nAbC/device-invites HTTP/1.1",
            "DELETE /api/v2/device-invites/12346 HTTP/1.1",
            "GET /api/v2/tailnet/-/acl HTTP/1.1",
            "POST /api/v2/tailnet/-/acl/validate HTTP/1.1",
            "POST /api/v2/tailnet/-/acl HTTP/1.1",
            "GET /api/v2/device/nAbC/device-invites HTTP/1.1",
        ]
    );
    for (_, head, _) in seen.iter() {
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer tskey-api-test\r\n"),
            "{head}"
        );
    }
    // One friend per invitation, never as an exit node.
    let body: Value = serde_json::from_str(&seen[0].2).unwrap();
    assert_eq!(body, json!([{ "multiUse": false, "allowExitNode": false }]));
    // The change applies only to the version read.
    assert!(seen[5].1.to_ascii_lowercase().contains("if-match: \"e1\""));
    let sent: Value = serde_json::from_str(&seen[5].2).unwrap();
    assert_eq!(sent, limited);
}
