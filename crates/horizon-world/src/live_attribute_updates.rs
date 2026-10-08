//! LivingEntity query/refresh dispatch over registered live attribute storage.
use crate::{
    attribute_map::ValueKind,
    attribute_updates::{self, UpdateWorld, UpdatedAttribute},
    attributes::live::{Error, Handle, Key, LiveAttributes},
    object_open_set::CursorError,
};

/// Entity handles keep a captured map alive even if a callback changes the
/// entity's current attributes. Map and instance accessors are direct storage
/// reads; virtual entity methods remain explicit dispatch points.
pub trait LiveAttributeWorld: UpdateWorld<Key> {
    type Identity: FnMut() -> i32;
    fn get_attributes(&mut self) -> Result<Self::Attributes, Self::Error>;
    fn live_attributes(&self, map: Self::Attributes) -> &LiveAttributes<Self::Identity>;
    fn live_attributes_mut(&mut self, map: Self::Attributes)
    -> &mut LiveAttributes<Self::Identity>;
    /// Resolve the queued instance itself, including an instance from another
    /// map if a caller has explicitly placed it in this map's pending set.
    fn instance_attribute(&self, instance: Option<Handle>) -> Result<Key, Self::Error>;
    fn attribute_error(error: Error) -> Self::Error;
    fn cursor_error(error: CursorError) -> Self::Error;
    fn dispatch_attribute_updated(&mut self, attribute: Key) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        attribute_updates::updated(self, attribute)
    }
}

/// Registered holder names are unique in a catalog. This classifier is for
/// registered attributes; custom holder dispatch can implement UpdateWorld.
pub fn kind(attribute: &Key) -> UpdatedAttribute {
    match attribute.name() {
        "minecraft:max_health" => UpdatedAttribute::MaxHealth,
        "minecraft:max_absorption" => UpdatedAttribute::MaxAbsorption,
        "minecraft:scale" => UpdatedAttribute::Scale,
        "minecraft:waypoint_transmit_range" => UpdatedAttribute::WaypointRange,
        _ => UpdatedAttribute::Other,
    }
}

/// Base body of getAttributeValue/getAttributeBaseValue, including the virtual
/// getAttributes call. Querying defaults does not create instances.
pub fn value<W: LiveAttributeWorld>(
    world: &mut W,
    attribute: Option<Key>,
    kind: ValueKind,
) -> Result<f64, W::Error> {
    let map = world.get_attributes()?;
    value_in(world, map, attribute, kind)
}

/// Query an already captured map, as used by waypoint updates after their
/// direct attributes-field read. This does not call virtual getAttributes.
pub fn value_in<W: LiveAttributeWorld>(
    world: &mut W,
    map: W::Attributes,
    attribute: Option<Key>,
    kind: ValueKind,
) -> Result<f64, W::Error> {
    world
        .live_attributes_mut(map)
        .value(attribute, kind)
        .map_err(W::attribute_error)
}

/// Base private refresh body for concrete registered AttributeMaps. Callbacks
/// may change map selection or mutate the captured pending set. Clear that set
/// only after the live cursor and every attribute callback finish successfully.
pub fn refresh<W: LiveAttributeWorld>(world: &mut W) -> Result<(), W::Error> {
    let map = world.get_attributes()?;
    let mut cursor = world.live_attributes(map).updates().cursor();
    while cursor.has_next() {
        let instance = world
            .live_attributes(map)
            .updates()
            .next(&mut cursor)
            .map_err(W::cursor_error)?;
        let attribute = world.instance_attribute(instance)?;
        world.dispatch_attribute_updated(attribute)?;
    }
    world.live_attributes_mut(map).updates_mut().clear();
    Ok(())
}
