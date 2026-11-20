//! The mods' say over a login connection's handshake (`player_handshake`). A connection thread
//! has no game to run a hook in, so it queues what the client sent here and waits; the game
//! thread answers each tick.
use horizon_core::nbt::Tag;
use std::{
    net::IpAddr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

/// How long a connection waits for the mods; a connection the server thread has not answered
/// by then is refused.
pub const WAIT: Duration = Duration::from_secs(5);

/// The longest host a mod may leave (`handshake::PROXY_HOST_LIMIT`, what a proxy may send).
pub const HOST_LIMIT: usize = horizon_protocol::packets::handshake::PROXY_HOST_LIMIT;

/// What the mods are asked about one connection.
pub struct Ask {
    id: u64,
    /// The peer's IP; None for the integrated server's own player.
    pub address: Option<IpAddr>,
    pub host: String,
    pub port: u16,
    pub protocol: i32,
    reply: mpsc::SyncSender<Answer>,
}

/// What the mods left of it.
pub struct Answer {
    pub host: String,
    /// The disconnect reason, when a mod refused the connection.
    pub refusal: Option<Tag>,
}

impl Ask {
    /// Hand the mods' answer to the waiting connection.
    pub fn answer(self, mut answer: Answer) {
        if answer.host.chars().count() > HOST_LIMIT {
            answer.host = answer.host.chars().take(HOST_LIMIT).collect();
        }
        let _ = self.reply.send(answer);
    }
}

#[derive(Default)]
pub struct Gate {
    listened: AtomicBool,
    next: AtomicU64,
    queue: Mutex<Vec<Ask>>,
}

impl Gate {
    /// Whether an enabled mod hears the hook; the game sets it each tick.
    pub fn set_listened(&self, on: bool) {
        self.listened.store(on, Ordering::Relaxed);
    }

    /// The questions waiting for the game thread, oldest first.
    pub fn take(&self) -> Vec<Ask> {
        std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Ask the mods about a connection: the host the login goes on with, or the refusal. Costs
    /// one atomic load when no mod listens.
    pub fn pass(
        &self,
        address: Option<IpAddr>,
        host: &str,
        port: u16,
        protocol: i32,
        wait: Duration,
        timed_out: impl FnOnce() -> Tag,
    ) -> Result<String, Tag> {
        if !self.listened.load(Ordering::Relaxed) {
            return Ok(host.to_owned());
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Ask {
                id,
                address,
                host: host.to_owned(),
                port,
                protocol,
                reply,
            });
        match answer.recv_timeout(wait) {
            Ok(Answer {
                refusal: Some(reason),
                ..
            }) => Err(reason),
            Ok(Answer { host, .. }) => Ok(host),
            Err(_) => {
                // Nobody will answer a connection that is gone: do not leave it queued.
                self.queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|ask| ask.id != id);
                Err(timed_out())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn nbt(text: &str) -> Tag {
        Tag::String(text.into())
    }

    /// With no mod listening the host passes untouched and nothing is queued.
    #[test]
    fn nothing_is_asked_when_no_mod_listens() {
        let gate = Gate::default();
        let host = gate.pass(None, "play.example\0data", 25565, 775, WAIT, || nbt("late"));
        assert_eq!(host, Ok("play.example\0data".to_owned()));
        assert!(gate.take().is_empty());
    }

    /// The game thread sees what the client sent and its answer is what the connection gets back.
    #[test]
    fn the_mods_rewrite_the_host_and_refuse() {
        let gate = Arc::new(Gate::default());
        gate.set_listened(true);
        let game = {
            let gate = gate.clone();
            std::thread::spawn(move || {
                let mut seen = Vec::new();
                while seen.len() < 2 {
                    for ask in gate.take() {
                        seen.push((ask.host.clone(), ask.port, ask.protocol));
                        let refuse = ask.host.starts_with("bad");
                        let plain = ask.host.split('\0').next().unwrap().to_owned();
                        ask.answer(Answer {
                            host: plain,
                            refusal: refuse.then(|| nbt("no")),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                seen
            })
        };
        let ip = Some("203.0.113.9".parse().unwrap());
        let ok = gate.pass(ip, "play.example\0abc", 25565, 775, WAIT, || nbt("late"));
        let refused = gate.pass(ip, "bad\0abc", 25566, 774, WAIT, || nbt("late"));
        assert_eq!(ok, Ok("play.example".to_owned()));
        assert_eq!(refused, Err(nbt("no")));
        assert_eq!(
            game.join().unwrap(),
            vec![
                ("play.example\0abc".to_owned(), 25565, 775),
                ("bad\0abc".to_owned(), 25566, 774),
            ]
        );
    }

    /// No answer in time refuses the connection and takes its question back.
    #[test]
    fn no_answer_refuses_and_dequeues() {
        let gate = Gate::default();
        gate.set_listened(true);
        let out = gate.pass(None, "h", 1, 2, Duration::from_millis(30), || nbt("late"));
        assert_eq!(out, Err(nbt("late")));
        assert!(gate.take().is_empty());
    }

    /// A host a mod makes longer than a proxy may send is cut.
    #[test]
    fn a_long_rewritten_host_is_cut() {
        let gate = Arc::new(Gate::default());
        gate.set_listened(true);
        let game = {
            let gate = gate.clone();
            std::thread::spawn(move || {
                loop {
                    if let Some(ask) = gate.take().into_iter().next() {
                        ask.answer(Answer {
                            host: "a".repeat(HOST_LIMIT + 5),
                            refusal: None,
                        });
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        let host = gate.pass(None, "h", 1, 2, WAIT, || nbt("late")).unwrap();
        game.join().unwrap();
        assert_eq!(host.len(), 32767);
    }
}
