use super::*;

impl Game<'_> {
    /// ServerPlayerGameMode.handleBlockBreakAction, STOP_DESTROY_BLOCK: the client finished
    /// digging `pos` (and predicted the break). The server breaks it if its own count of the
    /// digging since the START that named `pos` is at least 70 % done; otherwise it keeps digging
    /// alone and breaks it when done (`guest_delayed_destroy`), so the client's broken block is
    /// never left standing on the server.
    pub(super) fn guest_stop_digging(&mut self, conn: u32, pos: [i32; 3]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let ticks = host.hub.ticks;
        let air = self
            .res
            .facts()
            .is_air(self.world.block(pos[0], pos[1], pos[2]));
        let per_tick = if !air
            && host
                .play
                .get(&conn)
                .is_some_and(|p| p.dig_start_for(pos).is_some())
        {
            self.as_guest(conn, |g| g.destroy_progress(pos))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let decision = {
            let play = host.play.entry(conn).or_default();
            play.dig_ended();
            stop_dig(play, pos, air, ticks, per_tick)
        };
        match decision {
            StopDig::Other => {}
            // Already broken (the guest's own tick dug it out): only the crack goes.
            StopDig::Gone => self.guest_crack_gone(conn, pos),
            StopDig::Break => self.guest_destroy(conn, pos),
            StopDig::Delay(start) => {
                let play = host.play.entry(conn).or_default();
                if play.delayed.is_none() {
                    play.delayed = Some((pos, start));
                }
            }
        }
    }

    /// ServerPlayerGameMode.tick's delayed destroy: the block breaks once its progress since
    /// the digging started is complete, unless something else removed it first.
    pub(super) fn guest_delayed_destroy(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some((pos, start)) = host.play.get(&conn).and_then(|p| p.delayed) else {
            return;
        };
        let ticks = host.hub.ticks;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let progress = if self.res.facts().is_air(state) {
            None
        } else {
            self.as_guest(conn, |g| g.destroy_progress(pos))
                .map(|p| p * (ticks - start + 1) as f32)
        };
        match progress {
            Some(p) if p < 1.0 => {}
            done => {
                if let Some(play) = self.lan.as_mut().and_then(|host| host.play.get_mut(&conn)) {
                    play.delayed = None;
                }
                if done.is_some() {
                    self.guest_destroy(conn, pos);
                }
            }
        }
    }

    /// ServerPlayerGameMode.destroyBlock for a guest, its crack cleared for the others
    /// (`destroyBlockProgress(id, pos, -1)`).
    pub(super) fn guest_destroy(&mut self, conn: u32, pos: [i32; 3]) {
        if let Some(gi) = self.guest_index(conn) {
            let _ = self.guest_destroy_at(gi, conn, pos);
        }
    }

    /// `guest_destroy` for the guest at `gi`.
    pub(super) fn guest_destroy_at(
        &mut self,
        gi: usize,
        conn: u32,
        pos: [i32; 3],
    ) -> Result<(), TurnRefused> {
        self.as_guest_at(gi, conn, |g| {
            let _ = g.break_block(pos);
        })?;
        self.guest_crack_gone(conn, pos);
        Ok(())
    }

    /// Guest `conn` (at `gi`) breaks the block at `pos` at once, as a fake player's `BreakBlock`
    /// does: `handleBlockBreakAction`'s reach, build height, spawn protection and border, then
    /// `ServerPlayerGameMode.destroyBlock`, with its game-mode and tool rules, the native mods'
    /// `break_block`, the tool's wear and the drops, and no digging time. Outside creative, a
    /// block no digging breaks (destroy time -1) stays. True when the block broke.
    pub(in crate::game) fn guest_break_block(
        &mut self,
        conn: u32,
        gi: usize,
        pos: [i32; 3],
    ) -> Result<bool, ActionRefused> {
        let Some(host) = self.lan.as_mut() else {
            return Err(ActionRefused::Stale);
        };
        if !host.hub.guests.update(gi, |g| g.may_reach_block(pos)) {
            return Err(ActionRefused::OutOfReach);
        }
        let (min_y, max_y) = (self.world.shape.min_y, self.world.shape.max_y() - 1);
        if !(min_y..=max_y).contains(&pos[1])
            || self.under_spawn_protection(gi, pos)
            || !self.border.contains(f64::from(pos[0]), f64::from(pos[2]))
        {
            return Err(ActionRefused::Protected);
        }
        let before = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts().is_air(before) {
            return Ok(false);
        }
        // A survival player reaches `destroyBlock` only through its digging, which makes no
        // progress on a block of destroy time -1 (bedrock, barriers, command blocks): only a
        // creative one breaks those.
        let creative = self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].creative());
        if !creative && self.res.facts().get(before).hardness < 0.0 {
            return Ok(false);
        }
        self.guest_destroy_at(gi, conn, pos)
            .map_err(|_| ActionRefused::Stale)?;
        Ok(self.world.block(pos[0], pos[1], pos[2]) != before)
    }

    /// ServerPlayerGameMode.incrementDestroyProgress, including the first hit.
    pub(super) fn guest_dig_feedback(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(play) = host.play.get_mut(&conn) else {
            return;
        };
        let Some((pos, _)) = play.digging else {
            return;
        };
        if play.last_progress >= 0
            && self
                .res
                .facts()
                .is_air(self.world.block(pos[0], pos[1], pos[2]))
        {
            play.last_progress = -1;
            self.guest_crack_gone(conn, pos);
            return;
        }
        let Some(mining) = play.mining.as_ref() else {
            return;
        };
        let progress =
            (mining.per_tick * (host.hub.ticks - play.dig_start + 1) as f32 * 10.0) as i32;
        if progress != play.last_progress && progress >= 0 {
            play.last_progress = progress;
            if mod_api::__private::listened(mod_api::HookId::BlockBreakProgressUpdate) {
                let Some(guest) = host.hub.guest(conn) else {
                    return;
                };
                let (uuid, name) = (guest.uuid, guest.name.clone());
                self.native_block_break_progress_update(uuid, &name, pos, progress);
            }
            self.guest_crack_progress(conn, pos, progress);
        }
    }

    /// `level.destroyBlockProgress(id, pos, -1)` for a guest: its crack on `pos` is gone, for the
    /// host and the other guests.
    /// A player who dies digs and uses nothing any more: the block it was
    /// breaking, or had left to break (`hasDelayedDestroy`), stays, and its
    /// crack goes. Respawning gives it a new ServerPlayerGameMode (arismoko/horizon#1315).
    pub(super) fn guest_forget_digging(&mut self, conn: u32) {
        let Some(play) = self.lan.as_mut().and_then(|host| host.play.get_mut(&conn)) else {
            return;
        };
        let cracked = play.digging.take().filter(|_| play.last_progress >= 0);
        play.mining = None;
        play.delayed = None;
        // Respawning gives it a new ServerPlayerGameMode: the clock goes with the body.
        play.destroy_pos = None;
        play.destroy_delay = 0;
        play.using = None;
        play.last_progress = -1;
        if let Some((pos, _)) = cracked {
            self.guest_crack_gone(conn, pos);
        }
    }
    pub(super) fn guest_crack_gone(&mut self, conn: u32, pos: [i32; 3]) {
        self.guest_crack_progress(conn, pos, -1);
    }
    /// Only other players strictly within 32 blocks receive crack updates.
    pub(super) fn guest_crack_progress(&mut self, conn: u32, pos: [i32; 3], progress: i32) {
        let Some(id) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.entity_id)
        else {
            return;
        };
        self.destroy_block_progress(id, pos, progress);
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        host.hub.broadcast_near(
            [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
            32.0,
            &C::BlockDestruction(play::BlockDestruction {
                id,
                pos,
                progress: progress as u8,
            }),
            Some(conn),
        );
    }
}
