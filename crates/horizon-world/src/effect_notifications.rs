//! Base LivingEntity effect eligibility, attribute refresh and passenger updates.
use crate::{
    active_effects::{ActiveEffects, EffectHooks},
    java_hash_map::JavaKey,
    object_open_set::{CursorError, ObjectKey, ObjectOpenSet},
    status_effect::EffectInstance,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImmunityTag {
    Infested,
    Oozing,
    PoisonAndRegeneration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestrictedEffect {
    Infested,
    Oozing,
    Regeneration,
    Poison,
}
pub trait EligibilityWorld<H> {
    type Error;
    fn has_immunity_tag(
        &mut self,
        storage: &mut ActiveEffects<H>,
        tag: ImmunityTag,
    ) -> Result<bool, Self::Error>;
    fn is_restricted_effect(&self, effect: &H, kind: RestrictedEffect) -> bool;
}
pub fn can_be_affected<H, W: EligibilityWorld<H>>(
    storage: &mut ActiveEffects<H>,
    instance: &EffectInstance<H>,
    world: &mut W,
) -> Result<bool, W::Error> {
    // These branches are exclusive, even when the actor belongs to several tags.
    if world.has_immunity_tag(storage, ImmunityTag::Infested)? {
        Ok(!world.is_restricted_effect(&instance.read().effect, RestrictedEffect::Infested))
    } else if world.has_immunity_tag(storage, ImmunityTag::Oozing)? {
        Ok(!world.is_restricted_effect(&instance.read().effect, RestrictedEffect::Oozing))
    } else if world.has_immunity_tag(storage, ImmunityTag::PoisonAndRegeneration)? {
        let data = instance.read();
        Ok(
            !world.is_restricted_effect(&data.effect, RestrictedEffect::Regeneration)
                && !world.is_restricted_effect(&data.effect, RestrictedEffect::Poison),
        )
    } else {
        Ok(true)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectPacket<H> {
    Update {
        entity: i32,
        effect: H,
        amplifier: i32,
        duration: i32,
        flags: u8,
    },
    Remove {
        entity: i32,
        effect: H,
    },
}
impl<H: Clone> EffectPacket<H> {
    pub fn update(entity: i32, instance: &EffectInstance<H>, blend: bool) -> Self {
        let data = instance.read();
        Self::Update {
            entity,
            effect: data.effect.clone(),
            amplifier: data.details.amplifier,
            duration: data.details.duration,
            flags: u8::from(data.details.ambient)
                | (u8::from(data.details.visible) << 1)
                | (u8::from(data.details.show_icon) << 2)
                | (u8::from(blend) << 3),
        }
    }
}
/// Attribute maps, pending-update sets and connections are stable handles. A
/// callback may switch the actor's current handle without changing a captured one.
pub trait NotificationWorld<H: JavaKey + Clone>: EffectHooks<H> {
    type Attributes: Copy;
    type UpdateSet: Copy;
    type AttributeInstance: ObjectKey + Clone;
    type Attribute;
    type Passenger: Copy;
    type Connection: Copy;
    fn attributes(
        &mut self,
        storage: &mut ActiveEffects<H>,
    ) -> Result<Self::Attributes, Self::Error>;
    fn add_modifiers(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        attributes: Self::Attributes,
        amplifier: i32,
    ) -> Result<(), Self::Error>;
    fn remove_modifiers(
        &mut self,
        storage: &mut ActiveEffects<H>,
        effect: H,
        attributes: Self::Attributes,
    ) -> Result<(), Self::Error>;
    fn pending_updates(
        &mut self,
        storage: &mut ActiveEffects<H>,
        attributes: Self::Attributes,
    ) -> Result<Self::UpdateSet, Self::Error>;
    fn update_set(&self, set: Self::UpdateSet) -> &ObjectOpenSet<Self::AttributeInstance>;
    fn update_set_mut(
        &mut self,
        set: Self::UpdateSet,
    ) -> &mut ObjectOpenSet<Self::AttributeInstance>;
    fn iterator_error(error: CursorError) -> Self::Error;
    /// None preserves a native null member's failure at getAttribute.
    fn attribute_of(
        &mut self,
        storage: &mut ActiveEffects<H>,
        instance: Option<Self::AttributeInstance>,
    ) -> Result<Self::Attribute, Self::Error>;
    fn attribute_updated(
        &mut self,
        storage: &mut ActiveEffects<H>,
        attribute: Self::Attribute,
    ) -> Result<(), Self::Error>;
    /// Entity.getPassengers is final and returns its current immutable list.
    fn passengers(&self) -> Vec<Self::Passenger>;
    fn is_server_player(&self, passenger: Self::Passenger) -> bool;
    /// Direct ServerPlayer.connection field access before the virtual getId call.
    fn connection(&self, passenger: Self::Passenger) -> Self::Connection;
    fn entity_id(&mut self, storage: &mut ActiveEffects<H>) -> Result<i32, Self::Error>;
    fn send_packet(
        &mut self,
        storage: &mut ActiveEffects<H>,
        connection: Self::Connection,
        packet: EffectPacket<H>,
    ) -> Result<(), Self::Error>;
    fn send_effect_to_passengers(
        &mut self,
        storage: &mut ActiveEffects<H>,
        instance: &EffectInstance<H>,
    ) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        send_to_passengers(storage, instance, self)
    }
}
pub fn send_to_passengers<H: JavaKey + Clone, W: NotificationWorld<H>>(
    storage: &mut ActiveEffects<H>,
    instance: &EffectInstance<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    for passenger in world.passengers() {
        if world.is_server_player(passenger) {
            let connection = world.connection(passenger);
            let entity = world.entity_id(storage)?;
            let packet = EffectPacket::update(entity, instance, false);
            world.send_packet(storage, connection, packet)?;
        }
    }
    Ok(())
}
pub fn refresh_attributes<H: JavaKey + Clone, W: NotificationWorld<H>>(
    storage: &mut ActiveEffects<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    let attributes = world.attributes(storage)?;
    let updates = world.pending_updates(storage, attributes)?;
    let mut cursor = world.update_set(updates).cursor();
    while cursor.has_next() {
        let instance = world
            .update_set(updates)
            .next(&mut cursor)
            .map_err(W::iterator_error)?;
        let attribute = world.attribute_of(storage, instance)?;
        world.attribute_updated(storage, attribute)?;
    }
    world.update_set_mut(updates).clear();
    Ok(())
}
pub fn on_added<H: JavaKey + Clone, W: NotificationWorld<H>>(
    storage: &mut ActiveEffects<H>,
    instance: &EffectInstance<H>,
    world: &mut W,
) -> Result<(), W::Error> {
    let level = world.level(storage)?;
    if !world.is_client(storage, level)? {
        storage.dirty = true;
        let effect = instance.read().effect.clone();
        let attributes = world.attributes(storage)?;
        let amplifier = instance.read().details.amplifier;
        world.add_modifiers(storage, effect, attributes, amplifier)?;
        world.send_effect_to_passengers(storage, instance)?;
    }
    Ok(())
}
pub fn on_updated<H: JavaKey + Clone, W: NotificationWorld<H>>(
    storage: &mut ActiveEffects<H>,
    instance: &EffectInstance<H>,
    reapply: bool,
    world: &mut W,
) -> Result<(), W::Error> {
    let level = world.level(storage)?;
    if world.is_client(storage, level)? {
        return Ok(());
    }
    storage.dirty = true;
    if reapply {
        let effect = instance.read().effect.clone();
        let attributes = world.attributes(storage)?;
        world.remove_modifiers(storage, effect.clone(), attributes)?;
        let attributes = world.attributes(storage)?;
        let amplifier = instance.read().details.amplifier;
        world.add_modifiers(storage, effect, attributes, amplifier)?;
        refresh_attributes(storage, world)?;
    }
    world.send_effect_to_passengers(storage, instance)
}
pub fn on_removed<H: JavaKey + Clone, W: NotificationWorld<H>>(
    storage: &mut ActiveEffects<H>,
    instances: &[EffectInstance<H>],
    world: &mut W,
) -> Result<(), W::Error> {
    let level = world.level(storage)?;
    if world.is_client(storage, level)? {
        return Ok(());
    }
    storage.dirty = true;
    for instance in instances {
        let effect = instance.read().effect.clone();
        let attributes = world.attributes(storage)?;
        world.remove_modifiers(storage, effect, attributes)?;
        for passenger in world.passengers() {
            if world.is_server_player(passenger) {
                let connection = world.connection(passenger);
                let entity = world.entity_id(storage)?;
                let effect = instance.read().effect.clone();
                world.send_packet(storage, connection, EffectPacket::Remove { entity, effect })?;
            }
        }
    }
    refresh_attributes(storage, world)
}
