//! Dispatch placement intents only after the shared connection boundary admitted them.
use super::*;
use horizon_client::recipe_book::placement_sync as sync;
impl Game<'_> {
    pub(super) fn guest_mod_book_placement(&mut self, conn: u32, frame: sync::Frame) {
        match frame {
            sync::Frame::Hello { epoch, schema } => {
                let actual = self
                    .native_net
                    .book_connections
                    .get(&conn)
                    .and_then(|connection| connection.authority.as_ref())
                    .map(|authority| authority.epoch);
                let response = if actual == Some(epoch) {
                    self.native_net
                        .placement
                        .servers
                        .entry(conn)
                        .or_insert_with(|| super::native_book_placement::Server::new(epoch))
                        .peer
                        .hello(epoch, schema)
                } else {
                    Err(sync::Refusal::Stale)
                };
                let response = response.unwrap_or_else(|reason| sync::Frame::Refused {
                    epoch,
                    number: 0,
                    window: 0,
                    nonce: 0,
                    reason,
                });
                self.send_guest_mod_book_placement(conn, response);
            }
            request @ sync::Frame::Catalog {
                epoch,
                number,
                window,
                nonce,
                ..
            } => match self.server_mod_book_catalogue(conn, request) {
                Ok((start, bytes)) => {
                    self.send_guest_mod_book_placement(conn, start);
                    let mut offset = 0;
                    for part in bytes.chunks(sync::CHUNK_BYTES) {
                        self.send_guest_mod_book_placement(
                            conn,
                            sync::Frame::Part {
                                epoch,
                                number,
                                offset,
                                bytes: part.to_vec(),
                            },
                        );
                        offset += part.len() as u32;
                    }
                }
                Err(reason) => self.send_guest_mod_book_placement(
                    conn,
                    sync::Frame::Refused {
                        epoch,
                        number,
                        window,
                        nonce,
                        reason,
                    },
                ),
            },
            sync::Frame::Place { selection, use_max } => {
                let outcome = self.server_mod_book_place(conn, &selection, use_max);
                // A policy or root-commit callback may have closed or replaced the menu.
                if self
                    .mod_book_menu_snapshot(
                        conn,
                        selection.catalog.window,
                        selection.catalog.menu.as_str(),
                    )
                    .is_ok_and(|now| now.instance == selection.catalog.instance)
                {
                    self.send_guest_mod_book_placement(
                        conn,
                        sync::Frame::Reply { selection, outcome },
                    );
                }
            }
            _ => {}
        }
    }
}
