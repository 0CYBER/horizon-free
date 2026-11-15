//! Stack count admission at the mod API conversion boundary.
use super::*;

/// Validate an output while retaining its count for a later placement policy.
/// Item identity, components, maxima and damage retain the ordinary write checks.
pub(crate) trait TryIntoEnginePreservingCount {
    type Engine;
    fn try_into_engine_preserving_count(
        &self,
        items: &Items,
    ) -> Result<Self::Engine, StackWriteError>;
}

impl TryIntoEnginePreservingCount for ApiStack {
    type Engine = EngineStack;

    fn try_into_engine_preserving_count(
        &self,
        items: &Items,
    ) -> Result<EngineStack, StackWriteError> {
        let mut single = self.clone();
        single
            .set_count(1)
            .map_err(|_| StackWriteError::EmptyEngineStack)?;
        let mut stack = single.try_into_engine(items)?;
        stack.count = self.count();
        Ok(stack)
    }
}

impl<T: TryIntoEnginePreservingCount> TryIntoEnginePreservingCount for [T] {
    type Engine = Vec<T::Engine>;

    fn try_into_engine_preserving_count(
        &self,
        items: &Items,
    ) -> Result<Self::Engine, StackWriteError> {
        self.iter()
            .map(|value| value.try_into_engine_preserving_count(items))
            .collect()
    }
}

impl TryIntoEngine for ApiStack {
    type Engine = EngineStack;
    type Error = StackWriteError;

    fn try_into_engine(&self, items: &Items) -> Result<EngineStack, Self::Error> {
        let id =
            mod_api::__private::item_key_id(self.item()).ok_or(StackWriteError::UnknownItem)?;
        let (id, definition) = items
            .items
            .get_key_value(id)
            .ok_or(StackWriteError::UnknownItem)?;
        let stack = encode_stack(self, id.clone())?;
        let effective = |name: &str, default: u32| -> Result<u32, StackWriteError> {
            match stack.data().and_then(|data| data.other(name)) {
                Some(Tag::Int(value)) => {
                    u32::try_from(*value).map_err(|_| StackWriteError::InvalidMaximum)
                }
                Some(_) => Err(StackWriteError::InvalidMaximum),
                None => Ok(default),
            }
        };
        let maximum = if stack.carries_component(Some(definition), "minecraft:max_stack_size") {
            effective("minecraft:max_stack_size", definition.max_stack)?
        } else {
            1
        };
        if !(1..=99).contains(&maximum) {
            return Err(StackWriteError::InvalidMaximum);
        }
        if !horizon_world::item_template::durability_allows_stack_size(
            stack.carries_component(Some(definition), "minecraft:max_damage"),
            i64::from(maximum),
        ) {
            return Err(StackWriteError::StackableDamageable);
        }
        if stack.count > maximum {
            return Err(StackWriteError::CountExceeded { maximum });
        }
        let maximum = if stack.carries_component(Some(definition), "minecraft:max_damage") {
            effective("minecraft:max_damage", definition.max_damage)?
        } else {
            0
        };
        let damage = self.damage_component().map_or_else(
            || {
                u32::try_from(stack.effective_damage(Some(definition))).map_err(|_| {
                    StackWriteError::InvalidComponents {
                        reason: mod_api::StackError::InvalidDamage,
                    }
                })
            },
            Ok,
        )?;
        if damage > maximum {
            return Err(StackWriteError::DamageExceeded { maximum });
        }
        if i32::try_from(damage).is_err() {
            return Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage,
            });
        }
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_counts_but_keeps_other_write_validation() {
        let items = super::super::stack_limit_tests::catalog();
        let key = |name| mod_api::__private::item_key(name).unwrap();
        let oversized = ApiStack::new(key("minecraft:stick"), u32::MAX).unwrap();
        assert_eq!(
            oversized
                .try_into_engine_preserving_count(&items)
                .unwrap()
                .count,
            u32::MAX
        );
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        let mut sword = ApiStack::new(key("minecraft:iron_sword"), 100).unwrap();
        sword.set_damage(11);
        assert_eq!(
            sword.try_into_engine_preserving_count(&items),
            Err(StackWriteError::DamageExceeded { maximum: 10 })
        );
        let mut malformed = oversized;
        malformed
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(-1))].into())
            .unwrap();
        assert_eq!(
            malformed.try_into_engine_preserving_count(&items),
            Err(StackWriteError::InvalidMaximum)
        );
    }
}
