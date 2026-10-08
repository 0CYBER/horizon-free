//! Effect modifier dispatch bound to a captured attribute map and its instances.
use crate::{
    attribute_map::AttributeMap,
    attributes::Modifier,
    effect_attribute_modifiers::{self as effects, Definitions, ModifierWorld},
    object_open_set::ObjectKey,
};

pub trait AttributeAccess<H: ObjectKey + Clone, I: ObjectKey + Clone> {
    type Error;
    /// Supplier creation receives the actual map, including pending queues,
    /// before the new instance is inserted into its lookup collection.
    fn create_instance(
        &mut self,
        definitions: &mut Definitions<H>,
        map: &mut AttributeMap<H, I>,
        attribute: Option<H>,
    ) -> Result<Option<I>, Self::Error>;
    fn get_instance(
        &mut self,
        definitions: &mut Definitions<H>,
        map: &mut AttributeMap<H, I>,
        attribute: Option<H>,
    ) -> Result<Option<I>, Self::Error> {
        map.get_instance(attribute, |map, key| {
            self.create_instance(definitions, map, key)
        })
    }
    fn remove_modifier(
        &mut self,
        definitions: &mut Definitions<H>,
        map: &mut AttributeMap<H, I>,
        instance: &I,
        id: &str,
    ) -> Result<(), Self::Error>;
    fn add_permanent(
        &mut self,
        definitions: &mut Definitions<H>,
        map: &mut AttributeMap<H, I>,
        instance: &I,
        modifier: Modifier,
    ) -> Result<(), Self::Error>;
}
/// The caller captures this map before dispatching the effect's modifier body.
/// Instance methods continue to receive it after lookup membership changes.
pub struct StoredAttributes<'a, H, I, W> {
    pub map: &'a mut AttributeMap<H, I>,
    pub world: &'a mut W,
}
impl<H: ObjectKey + Clone, I: ObjectKey + Clone, W: AttributeAccess<H, I>> ModifierWorld<H>
    for StoredAttributes<'_, H, I, W>
{
    type Instance = I;
    type Error = W::Error;
    fn get_instance(
        &mut self,
        definitions: &mut Definitions<H>,
        attribute: Option<H>,
    ) -> Result<Option<I>, Self::Error> {
        self.world.get_instance(definitions, self.map, attribute)
    }
    fn remove_modifier(
        &mut self,
        definitions: &mut Definitions<H>,
        instance: &I,
        id: &str,
    ) -> Result<(), Self::Error> {
        self.world
            .remove_modifier(definitions, self.map, instance, id)
    }
    fn add_permanent(
        &mut self,
        definitions: &mut Definitions<H>,
        instance: &I,
        modifier: Modifier,
    ) -> Result<(), Self::Error> {
        self.world
            .add_permanent(definitions, self.map, instance, modifier)
    }
}
pub fn add<H: ObjectKey + Clone, I: ObjectKey + Clone, W: AttributeAccess<H, I>>(
    definitions: &mut Definitions<H>,
    map: &mut AttributeMap<H, I>,
    world: &mut W,
    amplifier: i32,
) -> Result<(), effects::Error<W::Error>> {
    effects::add_modifiers(definitions, &mut StoredAttributes { map, world }, amplifier)
}
pub fn remove<H: ObjectKey + Clone, I: ObjectKey + Clone, W: AttributeAccess<H, I>>(
    definitions: &mut Definitions<H>,
    map: &mut AttributeMap<H, I>,
    world: &mut W,
) -> Result<(), effects::Error<W::Error>> {
    effects::remove_modifiers(definitions, &mut StoredAttributes { map, world })
}
