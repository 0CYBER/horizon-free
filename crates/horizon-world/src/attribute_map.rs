//! Live AttributeMap instance lookup and update/synchronization queue ownership.
use crate::{
    object_open_map::ObjectOpenMap,
    object_open_set::{ObjectKey, ObjectOpenSet},
};

/// Instance handles retain identity independently of map membership. In
/// particular, a failed creation callback can leave a queued instance that was
/// never inserted into `instances`.
pub struct AttributeMap<H, I> {
    pub instances: ObjectOpenMap<H, I>,
    pub updates: ObjectOpenSet<I>,
    pub sync: ObjectOpenSet<I>,
}
impl<H, I> Default for AttributeMap<H, I> {
    fn default() -> Self {
        Self {
            instances: ObjectOpenMap::default(),
            updates: ObjectOpenSet::default(),
            sync: ObjectOpenSet::default(),
        }
    }
}
impl<H: ObjectKey + Clone, I: ObjectKey + Clone> AttributeMap<H, I> {
    /// AttributeMap dispatches to Map.computeIfAbsent(Function), not fastutil's
    /// Object2ObjectFunction overload. A null mapping is absent for this call;
    /// the ordinary put re-finds its slot after the supplier callback returns.
    pub fn get_instance<E>(
        &mut self,
        key: Option<H>,
        create: impl FnOnce(&mut Self, Option<H>) -> Result<Option<I>, E>,
    ) -> Result<Option<I>, E> {
        if let Some(instance) = self.instances.get(key.as_ref()) {
            return Ok(Some(instance));
        }
        let instance = create(self, key.clone())?;
        if instance.is_some() {
            self.instances.put(key, instance.clone());
        }
        Ok(instance)
    }
    /// The callback performs instance.getAttribute().value().isClientSyncable().
    /// It runs after adding the update entry, and can inspect or mutate both
    /// queues. Failure preserves the update entry and skips the sync insertion.
    pub fn modified<E>(
        &mut self,
        instance: Option<I>,
        syncable: impl FnOnce(&mut Self, Option<I>) -> Result<bool, E>,
    ) -> Result<(), E> {
        self.updates.add(instance.clone());
        if syncable(self, instance.clone())? {
            self.sync.add(instance);
        }
        Ok(())
    }
}
