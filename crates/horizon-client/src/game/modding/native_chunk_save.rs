//! Full-chunk save seam. Event dispatch will be added separately.
use super::Game;
use horizon_client::chunk::Chunk;
use horizon_client::streaming::{ProtoSave, ProtoSnapshot};
use std::sync::Arc;

impl Game<'_> {
    /// Dispatch on the server thread with the exact full-chunk save copies.
    pub(super) fn native_full_chunk_snapshots(&mut self, chunks: &[Arc<Chunk>]) {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() {
            return;
        }
        for chunk in chunks {
            self.native_full_chunk_snapshot(chunk);
        }
    }

    pub(super) fn native_late_full_chunk_snapshot(&mut self, chunk: &Chunk) {
        if self.remote.is_none() && !mod_api::__private::mods().is_empty() {
            self.native_full_chunk_snapshot(chunk);
        }
    }

    fn native_full_chunk_snapshot(&mut self, _chunk: &Chunk) {}

    /// The proto stays checked out until all callbacks return and the save is submitted.
    pub(super) fn native_proto_snapshots(&mut self, protos: &[ProtoSave]) {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() {
            return;
        }
        let access = self.streaming.proto_access();
        let dimension = self.dim.id();
        for save in protos {
            let Some(status) = access.status(&save.proto) else {
                continue;
            };
            self.native_proto_snapshot(ProtoSnapshot {
                dimension,
                pos: [save.pos.0, save.pos.1],
                proto: &save.proto,
                access: access.as_ref(),
                status,
            });
        }
    }

    fn native_proto_snapshot(&mut self, _snapshot: ProtoSnapshot<'_>) {}
}
