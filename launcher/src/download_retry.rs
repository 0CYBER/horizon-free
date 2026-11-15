//! A request tried again when it failed before any of its body arrived: no connection, or a
//! server error (5xx). A response is only returned whole to the caller, so a retry never resumes
//! a body or reuses part of a file. A 4xx (a missing file, a rate limit) is final.
use std::time::Duration;

/// The waits before the second and the third try.
pub(crate) const WAITS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// `request` sent with [`WAITS`].
pub(crate) fn get(request: &ureq::Request) -> Result<ureq::Response, Box<ureq::Error>> {
    call(&WAITS, || request.clone().call().map_err(Box::new))
}

/// `request` once, then again after each of `waits` while it fails with no connection or a 5xx.
/// The error is boxed: ureq's is large.
fn call(
    waits: &[Duration],
    mut request: impl FnMut() -> Result<ureq::Response, Box<ureq::Error>>,
) -> Result<ureq::Response, Box<ureq::Error>> {
    let mut answer = request();
    for wait in waits {
        match &answer {
            Err(e) if matches!(**e, ureq::Error::Transport(_)) => {}
            Err(e) if matches!(**e, ureq::Error::Status(500.., _)) => {}
            _ => break,
        }
        std::thread::sleep(*wait);
        answer = request();
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A local server giving `answers[n]` to its `n`th connection (`None`: close it unanswered),
    /// then 200 to every later one; and `call` against it with zero waits: its status or body,
    /// and how many requests were made.
    fn run(answers: &'static [Option<&'static str>]) -> (Result<String, u16>, usize) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for n in 0.. {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.read(&mut [0; 4096]);
                if let Some(status) = answers.get(n).copied().unwrap_or(Some("200 OK")) {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody"
                    );
                }
            }
        });
        let agent = ureq::AgentBuilder::new().build();
        let mut requests = 0;
        let answer = call(&[Duration::ZERO; 2], || {
            requests += 1;
            agent.get(&url).call().map_err(Box::new)
        });
        let outcome = match answer.map_err(|e| *e) {
            Ok(response) => Ok(response.into_string().unwrap()),
            Err(ureq::Error::Status(code, _)) => Err(code),
            Err(ureq::Error::Transport(_)) => Err(0),
        };
        (outcome, requests)
    }

    #[test]
    fn a_dropped_connection_is_tried_again_and_the_body_returned() {
        assert_eq!(run(&[None]), (Ok("body".into()), 2));
    }

    #[test]
    fn server_errors_are_tried_three_times_in_all() {
        let failing = &[
            Some("503 Service Unavailable"),
            None,
            Some("502 Bad Gateway"),
            None,
        ];
        assert_eq!(run(failing), (Err(502), 3));
    }

    #[test]
    fn a_missing_file_a_rate_limit_or_a_refusal_is_final() {
        for (answer, code) in [
            (&[Some("404 Not Found")], 404),
            (&[Some("429 Too Many Requests")], 429),
            (&[Some("403 Forbidden")], 403),
        ] {
            assert_eq!(run(answer), (Err(code), 1), "{code}");
        }
    }
}
