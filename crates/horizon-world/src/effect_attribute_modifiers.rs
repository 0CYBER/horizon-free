//! MobEffect attribute templates and native modifier traversal/evaluation order.
use crate::{
    attributes::{Modifier, Operation},
    object_open_map::{ObjectOpenMap, TraversalError},
    object_open_set::{CursorError, ObjectKey},
};
use std::io;

/// A validated modifier definition. Each creation produces a fresh identity.
#[derive(Clone, Debug)]
pub struct Template(Modifier);
impl Template {
    pub fn new(id: &str, amount: f64, operation: Operation) -> io::Result<Self> {
        Modifier::new(id, amount, operation).map(Self)
    }
    pub fn id(&self) -> &str {
        self.0.id()
    }
    pub fn amount(&self) -> f64 {
        self.0.amount()
    }
    pub fn operation(&self) -> Operation {
        self.0.operation()
    }
    pub fn create(&self, amplifier: i32) -> Modifier {
        Modifier::new(
            self.id(),
            self.amount() * f64::from(amplifier.wrapping_add(1)),
            self.operation(),
        )
        .expect("template identifier already validated")
    }
}
pub type Definitions<H> = ObjectOpenMap<H, Template>;

#[derive(Debug)]
pub enum Error<E> {
    Cursor(CursorError),
    MissingTemplate,
    World(E),
}
impl<E> From<CursorError> for Error<E> {
    fn from(value: CursorError) -> Self {
        Self::Cursor(value)
    }
}
/// Virtual attribute methods may mutate the effect's definitions. Their
/// captured attribute-map receiver belongs to this world for the whole call.
pub trait ModifierWorld<H> {
    type Instance;
    type Error;
    fn get_instance(
        &mut self,
        definitions: &mut Definitions<H>,
        attribute: Option<H>,
    ) -> Result<Option<Self::Instance>, Self::Error>;
    fn remove_modifier(
        &mut self,
        definitions: &mut Definitions<H>,
        instance: &Self::Instance,
        id: &str,
    ) -> Result<(), Self::Error>;
    fn add_permanent(
        &mut self,
        definitions: &mut Definitions<H>,
        instance: &Self::Instance,
        modifier: Modifier,
    ) -> Result<(), Self::Error>;
}
pub fn add_definition<H: ObjectKey + Clone>(
    definitions: &mut Definitions<H>,
    attribute: Option<H>,
    template: Template,
) {
    definitions.put(attribute, Some(template));
}
pub fn remove_modifiers<H: ObjectKey + Clone, W: ModifierWorld<H>>(
    definitions: &mut Definitions<H>,
    world: &mut W,
) -> Result<(), Error<W::Error>> {
    let mut cursor = definitions.cursor();
    while cursor.has_next() {
        let entry = definitions.next_entry(&mut cursor)?;
        let key = definitions.entry_key(&entry)?;
        if let Some(instance) = world.get_instance(definitions, key).map_err(Error::World)? {
            let template = definitions
                .entry_value(&entry)?
                .ok_or(Error::MissingTemplate)?;
            world
                .remove_modifier(definitions, &instance, template.id())
                .map_err(Error::World)?;
        }
    }
    Ok(())
}
pub fn add_modifiers<H: ObjectKey + Clone, W: ModifierWorld<H>>(
    definitions: &mut Definitions<H>,
    world: &mut W,
    amplifier: i32,
) -> Result<(), Error<W::Error>> {
    let mut cursor = definitions.cursor();
    while cursor.has_next() {
        let entry = definitions.next_entry(&mut cursor)?;
        let key = definitions.entry_key(&entry)?;
        if let Some(instance) = world.get_instance(definitions, key).map_err(Error::World)? {
            let template = definitions
                .entry_value(&entry)?
                .ok_or(Error::MissingTemplate)?;
            world
                .remove_modifier(definitions, &instance, template.id())
                .map_err(Error::World)?;
            // removeModifier can invoke dirty callbacks that replace/rehash the
            // definitions. The indexed entry is read again afterwards.
            let template = definitions
                .entry_value(&entry)?
                .ok_or(Error::MissingTemplate)?;
            let modifier = template.create(amplifier);
            world
                .add_permanent(definitions, &instance, modifier)
                .map_err(Error::World)?;
        }
    }
    Ok(())
}
pub fn create_modifiers<H: ObjectKey + Clone, E>(
    definitions: &mut Definitions<H>,
    amplifier: i32,
    mut consumer: impl FnMut(&mut Definitions<H>, Option<H>, Modifier) -> Result<(), E>,
) -> Result<(), Error<E>> {
    definitions
        .for_each(|definitions, key, template| {
            let modifier = template.ok_or(Error::MissingTemplate)?.create(amplifier);
            consumer(definitions, key, modifier).map_err(Error::World)
        })
        .map_err(|error| match error {
            TraversalError::Cursor(e) => Error::Cursor(e),
            TraversalError::Callback(e) => e,
        })
}
