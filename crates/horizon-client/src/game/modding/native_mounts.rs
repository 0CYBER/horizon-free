//! Mounting for native mods (`native_entity_mount`: NeoForge's `EntityMountEvent`). Each place
//! the acting player or a mob gets on or off a vehicle asks `native_may_board` or
//! `native_may_leave` first, and does nothing when a mod says no.
use super::{Game, commands::Target, native_hooks::mob_kind};

impl Game<'_> {
    pub(super) fn retire_vehicle_registration(&self, target: Target) {
        match target {
            Target::Boat(id) => {
                if let Some(boat) = self.boats.iter().find(|boat| boat.id == id) {
                    boat.attachment_lifetime.retire();
                }
            }
            Target::Cart(id) => {
                if let Some(cart) = self.carts.iter().find(|cart| cart.id == id) {
                    cart.attachment_lifetime.retire();
                }
            }
            _ => {}
        }
    }

    /// `Entity.setRemoved` makes a vehicle invisible before notifying riders.
    /// The owning list is kept stable while callbacks run.
    pub(super) fn with_destroying_vehicle<R>(
        &mut self,
        target: Target,
        action: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if !self.persist.destroying_vehicles.insert(target) {
            return None;
        }
        self.retire_vehicle_registration(target);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self)));
        self.persist.destroying_vehicles.remove(&target);
        match result {
            Ok(value) => Some(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Whether this game is a server whose mounts the mods hear: the server thread or a
    /// dedicated server, never a client.
    fn native_hears_mounts(&self) -> bool {
        self.serving.is_some() || self.options.dedicated.is_some()
    }

    /// The entity id of the acting player: a guest's while their action runs (kept with the
    /// action, as the hub is often out of `lan` then), else none (0).
    pub(super) fn native_acting_entity(&self) -> u32 {
        match self.lan_acting {
            Some(conn) => self.lan_acting_entity.unwrap_or_else(|| {
                let guests = self.lan_guest_entities();
                guests.iter().find(|g| g.0 == conn).map_or(0, |g| g.1)
            }),
            None => 0,
        }
    }

    /// The type of the vehicle with entity id `id`.
    fn native_vehicle_kind(&self, id: u32) -> String {
        if let Some(b) = self.boat_object(id) {
            b.entity_type()
        } else if let Some(c) = self.cart_object(id) {
            c.kind.id().into()
        } else if let Some(vehicle) = self.persist.removed_vehicles.get(&id) {
            vehicle.kind()
        } else if self.cushions.iter().any(|c| c.id == id) {
            "minecraft:cushion".into()
        } else if let Some(m) = self.mob_object(id) {
            mob_kind(m)
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            self.target_type(Target::Stand(id))
        } else if self.others.iter().any(|o| o.id == id) {
            self.target_type(Target::Other(id))
        } else {
            String::new()
        }
    }

    /// `native_entity_mount` for `rider` (the acting player when `None`) and `vehicle`: whether
    /// the mounting or dismounting goes ahead.
    fn native_mount_allowed(&mut self, rider: Option<u32>, vehicle: u32, mounting: bool) -> bool {
        if !self.native_hears_mounts()
            || !mod_api::__private::listened(mod_api::HookId::EntityMount)
        {
            return true;
        }
        let entity = rider.unwrap_or_else(|| self.native_acting_entity());
        let notification = (entity, vehicle, mounting);
        // A callback can discard the vehicle before this edge is unlinked.
        // Its cleanup must not dispatch the identical in-flight request again.
        if !self.persist.mount_notifications.insert(notification) {
            return false;
        }
        let kind = match rider {
            Some(id) => self.mob_object(id).map_or_else(String::new, mob_kind),
            None => "minecraft:player".into(),
        };
        let vehicle_kind = self.native_vehicle_kind(vehicle);
        let mut canceled = false;
        self.native_entity_mount(mod_api::__private::EntityMountCall {
            dimension: self.dim.id(),
            entity,
            kind: &kind,
            vehicle,
            vehicle_kind: &vehicle_kind,
            mounting,
            canceled: &mut canceled,
        });
        self.persist.mount_notifications.remove(&notification);
        !canceled
    }

    /// The acting player is about to board `vehicle`: false when a mod canceled it.
    pub(super) fn native_may_board(&mut self, vehicle: u32) -> bool {
        // A fake player boards nothing: every boarding path asks here first.
        if self.fake_blocks(mod_api::FakeEffect::Ride) {
            return false;
        }
        self.native_mount_allowed(None, vehicle, true)
    }

    /// The acting player is about to get off `vehicle`: false when a mod canceled it.
    pub(super) fn native_may_leave(&mut self, vehicle: u32) -> bool {
        self.native_mount_allowed(None, vehicle, false)
    }

    /// The mob `rider` is about to board (`mounting`) or leave `vehicle`: false when a mod
    /// canceled it.
    pub(super) fn native_mob_may_ride(&mut self, rider: u32, vehicle: u32, mounting: bool) -> bool {
        self.native_mount_allowed(Some(rider), vehicle, mounting)
    }
}
