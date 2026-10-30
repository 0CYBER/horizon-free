use super::*;
use std::io::{BufRead, BufReader, Read, Write};

#[test]
fn search_matches_every_word_in_any_case() {
    assert!(matches(
        "[12:00:01] [Render thread/WARN]: Missing texture",
        "warn texture"
    ));
    assert!(!matches("[12:00:01] [Render thread/INFO]: Loaded", "warn"));
    assert!(matches("anything", "  "));
}

#[test]
fn the_shared_log_has_no_home_folder_or_names() {
    let log = "Loading /Users/ludovicmarie/Library/Application Support/horizon/options.txt\n\
               C:\\Users\\ludovicmarie\\AppData\\horizon too\n\
               Ludo47 joined the game\n\
               <ludo47> hi, Ludo470 is someone else\n\
               user LUDOVICMARIE on ludovicmarie-mac\n";
    let names = vec!["ludovicmarie".to_string(), "Ludo47".to_string()];
    let out = private(log, Some(Path::new("/Users/ludovicmarie")), &names);
    assert_eq!(
        out,
        "Loading ~/Library/Application Support/horizon/options.txt\n\
         C:~\\AppData\\horizon too\n\
         <player> joined the game\n\
         <<player>> hi, Ludo470 is someone else\n\
         user <player> on <player>-mac\n"
    );
}

#[test]
fn a_long_log_sends_its_end() {
    let log: String = (0..30_000).map(|i| format!("line {i}\n")).collect();
    let sent = fit(&log);
    assert_eq!(sent.lines().count(), MAX_LINES);
    assert!(sent.starts_with("line 5000\n") && sent.ends_with("line 29999\n"));
    assert_eq!(fit("short\n"), "short\n");
}

/// A one-request HTTP server that answers `body` and hands back what it was sent.
fn stub(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = n.trim().parse().unwrap();
            }
            head.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut sent = vec![0; length];
        reader.read_exact(&mut sent).unwrap();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        reader.get_mut().write_all(reply.as_bytes()).unwrap();
        head + &String::from_utf8(sent).unwrap()
    });
    (url, served)
}

#[test]
fn shares_and_deletes_through_the_paste_api() {
    let (url, served) =
        stub(r#"{"success":true,"id":"Ab12","url":"https://mclo.gs/Ab12","token":"t0k"}"#);
    std::env::set_var(PASTE_ENV, &url);
    let shared = share("[main/INFO]: hello\n").unwrap();
    assert_eq!(
        shared,
        Shared {
            url: "https://mclo.gs/Ab12".into(),
            id: "Ab12".into(),
            token: "t0k".into()
        }
    );
    let request = served.join().unwrap();
    assert!(request.starts_with("POST /log "), "{request}");
    assert!(
        request.ends_with(r#"{"content":"[main/INFO]: hello\n","source":"Horizon launcher"}"#),
        "{request}"
    );

    let (url, served) = stub(r#"{"success":true}"#);
    std::env::set_var(PASTE_ENV, &url);
    unshare(&shared).unwrap();
    let request = served.join().unwrap();
    assert!(request.starts_with("DELETE /log/Ab12 "), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer t0k"),
        "{request}"
    );

    let (url, _served) = stub(r#"{"success":false,"error":"Required field 'content' not found."}"#);
    std::env::set_var(PASTE_ENV, &url);
    assert_eq!(
        share("").unwrap_err(),
        "mclo.gs didn't take the log: Required field 'content' not found."
    );
    std::env::remove_var(PASTE_ENV);
}
