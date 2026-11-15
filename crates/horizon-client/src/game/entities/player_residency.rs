//! Specialized authoritative owners that are not homogeneous entity lists.
use super::{Game, commands::Target};

pub(super) enum SpecializedSource<'a> {
    Player(&'a horizon_client::motion::PlayerMotion),
    Guest(&'a horizon_client::lan::guest::Guest),
    Hook(&'a super::fishing::Hook),
}
use horizon_client::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner, Source,
        table::{Location, SourceEntity},
    },
};

impl SpecializedSource<'_> {
    fn id(&self) -> u32 {
        match self {
            Self::Player(player) => player.entity_id as u32,
            Self::Guest(guest) => guest.entity_id as u32,
            Self::Hook(hook) => hook.id(),
        }
    }
}

impl Game<'_> {
    pub(super) fn current_player_resident_location(&self) -> Option<Location> {
        match self.lan_acting {
            Some(conn) => self.command_guest(conn)?.residency.location(),
            None => self.resident_entities.local_player.location(),
        }
    }

    /// Engine UUID consumers retain a specialized owner's immutable identity
    /// even through a real loan. Public I access applies its policy beforehand.
    pub(super) fn known_resident_entity_uuid_at(
        &self,
        dimension: mod_api::DimensionKey,
        id: u32,
    ) -> Option<[i32; 4]> {
        let mut table = self.resident_entities.table.borrow_mut();
        let (at, _, details) =
            table.server_admitted_identity(id, |at| self.locate_server_resident_owner(at))?;
        let uuid = details.known_uuid;
        (at.owner.dimension == dimension).then_some(uuid).flatten()
    }

    pub(super) fn resident_target_id(&self, target: Target) -> Option<u32> {
        match target {
            Target::Player => match self.lan_acting {
                Some(conn) => self
                    .command_guest(conn)?
                    .residency
                    .identity()
                    .map(|identity| identity.id.get()),
                None => self
                    .resident_entities
                    .local_player
                    .identity()
                    .map(|identity| identity.id.get()),
            },
            Target::Guest(conn) => self
                .command_guest(conn)?
                .residency
                .identity()
                .map(|identity| identity.id.get()),
            _ => super::native_entities::target_id(target),
        }
    }

    pub(super) fn acting_player_join(&self) -> horizon_client::join_seq::JoinSeq {
        self.lan_acting
            .and_then(|conn| self.command_guest(conn))
            .map_or_else(
                || self.resident_entities.local_player.join(),
                |guest| guest.residency.join(),
            )
    }

    pub(super) fn install_fishing_hook(&mut self, mut hook: super::fishing::Hook) {
        hook.caster = self.lan_acting;
        hook.caster_generation = self.acting_player_join();
        self.hook.discard();
        self.bind_current_hook_residency();
        self.hook.replace(hook);
    }

    pub(super) fn remove_local_player_body(&mut self) {
        self.hook.discard();
        self.resident_entities.local_player.retire();
        self.player_state.attachments.retire();
        self.player = None;
    }

    pub(super) fn bind_local_player_residency(&mut self) {
        if self.lan_acting.is_some() {
            return;
        }
        let journal = self.resident_entities.table.borrow().producer();
        let Some(player) = &self.player else {
            self.resident_entities.local_player.retire();
            return;
        };
        self.resident_entities.local_player.bind(
            &journal,
            Owner {
                dimension: self.dim.key(),
                kind: Kind::Player,
            },
            player.entity_id as u32,
            Source::LocalPlayer,
            self.remote
                .as_ref()
                .filter(|remote| remote.logged_in)
                .map(|remote| horizon_client::lan::playerdata::uuid_ints(remote.profile()))
                .or(self.level.player_uuid),
        );
        if let Some(remote) = self.remote.as_ref().filter(|remote| remote.logged_in)
            && let Some(epoch) = self.native_lifecycle.epoch()
        {
            remote
                .session
                .payload_transport()
                .bind_player(mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, remote.entity_id as u32),
                    remote.profile(),
                ));
        }
    }

    pub(super) fn bind_current_hook_residency(&mut self) {
        let journal = self.resident_entities.table.borrow().producer();
        let (owner, source) = self
            .hook
            .resident_location()
            .map(|at| (at.owner, at.source))
            .unwrap_or_else(|| {
                let dimension = self
                    .current_player_resident_location()
                    .map_or(self.dim.key(), |at| at.owner.dimension);
                (
                    Owner {
                        dimension,
                        kind: Kind::Hook,
                    },
                    Source::Hook {
                        caster: self.lan_acting,
                    },
                )
            });
        self.hook.bind_residency_specialized(
            &journal,
            owner,
            |hook| (hook.id(), hook.join_seq()),
            source,
            None,
            Some(|hook| Some(hook.uuid)),
        );
    }

    pub(super) fn bind_specialized_resident_owners(&mut self, journal: &Journal) {
        self.resident_entities.local_player.rebind(journal);
        let body_id = self.player.as_ref().map(|player| player.entity_id as u32);
        if self.lan_acting.is_some()
            || self
                .resident_entities
                .local_player
                .identity()
                .is_some_and(|identity| Some(identity.id.get()) != body_id)
        {
            self.resident_entities.local_player.extract();
        }
        self.bind_current_hook_residency();
        if self.hook.resident_location().is_some_and(|at| {
            at.source
                != (Source::Hook {
                    caster: self.lan_acting,
                })
        }) {
            self.hook.begin_loan();
        }
        if let Some(host) = &mut self.lan {
            host.hub
                .guests
                .bind_residency(journal, |id| super::Dim::from_id(id).key());
            for &conn in &self.lan_guest_swaps {
                if let Some(index) = host.hub.guests.connection_index(conn) {
                    let guest = &host.hub.guests[index];
                    if self.lan_acting != Some(conn) || body_id != Some(guest.entity_id as u32) {
                        guest.residency.extract();
                    }
                }
            }
            for (&conn, play) in &mut host.play {
                let Some(index) = host.hub.guests.connection_index(conn) else {
                    continue;
                };
                let guest = &host.hub.guests[index];
                let (owner, source) = play
                    .hook
                    .resident_location()
                    .map(|at| (at.owner, at.source))
                    .unwrap_or((
                        Owner {
                            dimension: super::Dim::from_id(&guest.dimension).key(),
                            kind: Kind::Hook,
                        },
                        Source::Hook { caster: Some(conn) },
                    ));
                play.hook.bind_residency_specialized(
                    journal,
                    owner,
                    |hook| (hook.id(), hook.join_seq()),
                    source,
                    None,
                    Some(|hook| Some(hook.uuid)),
                );
            }
        }
    }

    pub(super) fn specialized_resident_source(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(SpecializedSource<'_>, Target)>> {
        let (source, id, target) = match (at.owner.kind, at.source) {
            (Kind::Player, Source::LocalPlayer) => {
                if self.lan_acting.is_some()
                    || self.resident_entities.local_player.location() != Some(at)
                {
                    return None;
                }
                let player = self.player.as_ref()?;
                if self.resident_entities.local_player.identity()?.id.get()
                    != player.entity_id as u32
                {
                    return None;
                }
                (
                    SpecializedSource::Player(player),
                    player.entity_id as u32,
                    Target::Player,
                )
            }
            (Kind::Player, Source::Guest(conn)) => {
                let host = self.lan.as_ref()?;
                let guest = &host.hub.guests[host.hub.guests.connection_index(conn)?];
                if guest.residency.location() != Some(at) || guest.released {
                    return None;
                }
                if self.lan_acting == Some(conn) {
                    let player = self.player.as_ref()?;
                    if player.entity_id != guest.entity_id {
                        return None;
                    }
                    (
                        SpecializedSource::Player(player),
                        guest.entity_id as u32,
                        Target::Player,
                    )
                } else {
                    if self.lan_guest_swaps.contains(&conn) {
                        return None;
                    }
                    (
                        SpecializedSource::Guest(guest),
                        guest.entity_id as u32,
                        Target::Guest(conn),
                    )
                }
            }
            (Kind::Hook, Source::Hook { caster }) => {
                let hook = match caster {
                    None => {
                        if self.lan_acting.is_some() {
                            return None;
                        }
                        let hook = self.hook.as_ref()?;
                        if hook.caster_generation != self.resident_entities.local_player.join() {
                            return None;
                        }
                        hook
                    }
                    Some(conn) => {
                        let host = self.lan.as_ref()?;
                        let guest = host
                            .hub
                            .guests
                            .connection_index(conn)
                            .map(|index| &host.hub.guests[index]);
                        let hook = if self.lan_acting == Some(conn) {
                            self.hook.as_ref()?
                        } else {
                            if self.lan_guest_swaps.contains(&conn) {
                                return None;
                            }
                            host.play.get(&conn)?.hook.as_ref()?
                        };
                        if guest
                            .is_some_and(|guest| hook.caster_generation != guest.residency.join())
                        {
                            return None;
                        }
                        hook
                    }
                };
                if hook.join_seq() != at.join || hook.caster != caster {
                    return None;
                }
                (
                    SpecializedSource::Hook(hook),
                    hook.id(),
                    Target::Hook(hook.id()),
                )
            }
            _ => return None,
        };
        Some(SourceEntity {
            id,
            value: (source, target),
        })
    }

    pub(super) fn with_specialized_resident_slot<R>(
        &mut self,
        at: Location,
        visit: impl FnOnce(&mut horizon_mod_runtime::attachments::AttachmentLifetime) -> R,
    ) -> super::entity_residency::SlotAccess<R> {
        use super::entity_residency::SlotAccess;
        use horizon_mod_runtime::attachments::HasAttachmentLifetime;
        match at.source {
            Source::LocalPlayer => {
                SlotAccess::Done(visit(self.player_state.attachments.attachment_lifetime()))
            }
            Source::Guest(conn) => {
                if self.lan_acting == Some(conn) {
                    return SlotAccess::Done(visit(
                        self.player_state.attachments.attachment_lifetime(),
                    ));
                }
                let Some(attachments) = self
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.guests.attachments_mut(conn))
                else {
                    return SlotAccess::Missing;
                };
                SlotAccess::Done(visit(attachments.attachment_lifetime()))
            }
            Source::Hook { caster } => {
                let hook = if caster == self.lan_acting {
                    self.hook.as_mut()
                } else {
                    caster.and_then(|conn| self.lan.as_mut()?.play.get_mut(&conn)?.hook.as_mut())
                };
                let Some(hook) = hook else {
                    return SlotAccess::Missing;
                };
                SlotAccess::Done(visit(hook.attachment_lifetime()))
            }
            Source::DragonPart(_) | Source::Ordinary => SlotAccess::Missing,
        }
    }

    pub(super) fn locate_specialized_resident_owner(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(Target, bool)>> {
        let source = self.specialized_resident_source(at)?;
        let (_, target) = source.value;
        Some(SourceEntity {
            id: source.value.0.id(),
            value: (
                target,
                self.resident_source_target_visible(at.owner.dimension, target),
            ),
        })
    }
}
