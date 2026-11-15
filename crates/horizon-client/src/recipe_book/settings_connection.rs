//! Negotiated server boundary: capability, replay state and response budget
//! belong to one actual player connection, never to a brand or global owner.
#[path = "settings_placement.rs"]
mod placement;
use super::{
    BookFlags, CustomBooks, settings_codec,
    settings_sync::{Authority, Message, RequestBudget},
};
use std::{collections::BTreeSet, time::Instant};

#[derive(Default)]
pub struct Connection {
    capability: bool,
    pub authority: Option<Authority>,
    budget: RequestBudget,
}
pub struct Reply {
    pub message: Message,
    pub changed: Option<(String, BookFlags)>,
}
impl Connection {
    /// Called only after resolving the actual placed player connection.
    /// Excess payloads (including malformed requests) produce no response.
    pub fn receive(
        &mut self,
        data: &[u8],
        books: &mut CustomBooks,
        registered: impl FnOnce() -> BTreeSet<String>,
        epoch: impl FnOnce() -> u64,
        now: Instant,
    ) -> Option<Reply> {
        if !self.budget.accept(now) {
            return self.capability.then_some(Reply {
                message: Message::Busy,
                changed: None,
            });
        }
        let message = settings_codec::decode(data)?;
        let mut changed = None;
        let message = match message {
            Message::Hello => {
                if self.capability {
                    return None;
                }
                self.capability = true;
                Message::Ready
            }
            Message::Offer(keys) => {
                if !self.capability || self.authority.is_some() {
                    return None;
                }
                let authority = Authority::negotiate(epoch(), keys, &registered());
                let snapshot = authority.snapshot(books);
                self.authority = Some(authority);
                snapshot
            }
            Message::Request {
                epoch,
                number,
                key,
                flags,
            } => {
                let (reply, accepted) = self
                    .authority
                    .as_mut()?
                    .request(books, epoch, number, &key, flags, now);
                if accepted {
                    changed = Some((key, flags));
                }
                reply
            }
            Message::Resnapshot => self.authority.as_ref()?.snapshot(books),
            _ => return None,
        };
        Some(Reply { message, changed })
    }
}

#[cfg(test)]
mod tests {
    use super::super::settings_codec::encode;
    use super::*;
    #[test]
    fn real_boundary_negotiates_before_snapshot_and_notifies_only_accepted_changes() {
        let registered: BTreeSet<String> = ["demo:pot".to_owned(), "demo:oven".to_owned()].into();
        let mut connection = Connection::default();
        let mut books = CustomBooks::default();
        let now = Instant::now();
        let offer = encode(&Message::Offer(vec!["demo:pot".into()])).unwrap();
        assert!(
            connection
                .receive(&offer, &mut books, || registered.clone(), || 7, now)
                .is_none()
        );
        let hello = encode(&Message::Hello).unwrap();
        assert_eq!(
            connection
                .receive(&hello, &mut books, || registered.clone(), || 7, now)
                .unwrap()
                .message,
            Message::Ready
        );
        assert!(
            matches!(connection.receive(&offer,&mut books,|| registered.clone(),||7,now).unwrap().message,Message::Snapshot { entries,.. } if entries==vec![("demo:pot".into(),BookFlags::default())])
        );
        let request = encode(&Message::Request {
            epoch: 7,
            number: 1,
            key: "demo:pot".into(),
            flags: BookFlags {
                open: true,
                filtering: true,
            },
        })
        .unwrap();
        assert_eq!(
            connection
                .receive(&request, &mut books, || registered.clone(), || 8, now)
                .unwrap()
                .changed,
            Some((
                "demo:pot".into(),
                BookFlags {
                    open: true,
                    filtering: true
                }
            ))
        );
        assert!(
            connection
                .receive(&request, &mut books, || registered.clone(), || 8, now)
                .unwrap()
                .changed
                .is_none()
        );
        assert_eq!(books.get("demo:oven"), BookFlags::default());
        // Repeated offers cannot reset epoch or replay counters.
        assert!(
            connection
                .receive(&offer, &mut books, || registered.clone(), || 8, now)
                .is_none()
        );
        assert_eq!(connection.authority.as_ref().unwrap().epoch, 7);
        let resnapshot = encode(&Message::Resnapshot).unwrap();
        for _ in 0..14 {
            assert!(
                connection
                    .receive(&resnapshot, &mut books, || registered.clone(), || 8, now)
                    .is_some()
            );
        }
        for _ in 0..100 {
            let reply = connection
                .receive(&resnapshot, &mut books, || registered.clone(), || 8, now)
                .unwrap();
            assert_eq!(reply.message, Message::Busy);
            assert!(reply.changed.is_none());
            assert_eq!(encode(&reply.message).unwrap(), vec![7]);
        }
        assert!(
            connection
                .receive(
                    &resnapshot,
                    &mut books,
                    || registered.clone(),
                    || 8,
                    now + std::time::Duration::from_secs(1)
                )
                .is_some()
        );
    }
}
