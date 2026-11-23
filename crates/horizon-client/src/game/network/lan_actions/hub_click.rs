//! A hub container click the game owns. `Hub::container_click` used to run inside
//! `self.lan.as_mut()`, so nothing it called could reach the game: a hook fired in the middle
//! of a click had no `&Game` to be given. Here the game lifts the host out of itself for the
//! click, so the click code and the game are separate values and the game is whole while the
//! hub runs.
use super::*;

impl Game<'_> {
    /// Run `f` with the LAN host taken out of `self.lan`, then put it back. `f` is given the whole
    /// game (`self.lan` is `None` while it runs) and the host. The host is restored on every
    /// path, including a panic in `f`, which is then propagated: no borrow of the host outlives
    /// the call and the game is not left without it. `None` when there is no host to lift,
    /// which includes a call made while the host is already lifted.
    pub(in crate::game) fn with_lan_lifted<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut LanHost) -> R,
    ) -> Option<R> {
        let mut host = self.lan.take()?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, &mut host)));
        debug_assert!(self.lan.is_none(), "a lifted LAN host was replaced");
        self.lan = Some(host);
        match result {
            Ok(result) => Some(result),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// The mods' `container_click` for a click in the guest's inventory menu (window 0) or open
    /// block container menu: true when a mod canceled it. The click then changes nothing and the
    /// whole menu with the cursor goes back to the client (`sendAllDataToRemote`). A native mod's
    /// menu, a drag (click type 5), a spectator's or a dead player's click and every window the hub
    /// does not run are not asked.
    pub(super) fn lan_container_click_vetoed(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        // ClickType.QUICK_CRAFT: a drag is told at no step.
        if p.input == 5 || !mod_api::__private::listened(mod_api::HookId::ContainerClick) {
            return false;
        }
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let Some(g) = host.hub.guest(conn) else {
            return false;
        };
        if g.spectator() || g.dead {
            return false;
        }
        let slot = i32::from(p.slot);
        let at = usize::try_from(slot).ok();
        let clicked = if p.container == 0 {
            at.and_then(|s| g.menu_sent.get(s).cloned().flatten())
        } else {
            match g.menu.as_ref().filter(|m| {
                m.window == p.container && m.slots != horizon_client::lan::menu::Slots::Native
            }) {
                Some(m) => at.and_then(|s| m.sent.get(s).cloned().flatten()),
                None => return false,
            }
        };
        let (uuid, carried) = (g.uuid, g.carried.clone());
        let mut canceled = false;
        let Ok(api_carried) = carried
            .as_ref()
            .map(crate::game::api_values::TryIntoApi::try_into_api)
            .transpose()
        else {
            return false;
        };
        let Ok(api_clicked) = clicked
            .as_ref()
            .map(crate::game::api_values::TryIntoApi::try_into_api)
            .transpose()
        else {
            return false;
        };
        self.as_guest(conn, |game| {
            game.native_container_click(mod_api::__private::ContainerClickCall {
                player: uuid,
                window: p.container,
                slot,
                button: i32::from(p.button),
                mode: p.input,
                carried: api_carried.as_ref(),
                clicked: api_clicked.as_ref(),
                canceled: &mut canceled,
            });
        });
        if !canceled {
            return false;
        }
        let Some(host) = self.lan.as_mut() else {
            return true;
        };
        if p.container == 0 {
            if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                host.hub.send_inventory(gi);
            }
        } else {
            host.hub.resend_menu(conn, p.container, &self.world);
        }
        true
    }

    /// `ServerGamePacketListenerImpl.handleContainerClick` for the guest `conn`: the hub's
    /// click, run with the host lifted out of the game. Returns the stacks the click threw out.
    pub(super) fn lan_hub_click(&mut self, conn: u32, p: &play::ContainerClick) -> Vec<ItemStack> {
        self.with_lan_lifted(|game, host| {
            // The game is lent to the click as the mods' `menu_slot_rules` (their read-only `Host`).
            // `self.lan` is `None` meanwhile: what a hook reads of the LAN host answers as if none
            // were open.
            let rules = ItemRules(
                world_items!(game),
                &world_stations!(game).enchantments,
                Some(&*game),
            );
            let Some(mut prepared) = host.hub.prepare_click(conn, p, &game.world) else {
                return Vec::new();
            };
            // The game is whole here: `game` can be read or written while the hub clicks.
            let outcome = host.hub.run_click(&mut prepared, p, &rules);
            host.hub.finish_click(prepared, p, &mut game.world, outcome)
        })
        .unwrap_or_default()
    }
}
