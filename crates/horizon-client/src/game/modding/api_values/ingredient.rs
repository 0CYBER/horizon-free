//! Ingredient conversions retain concrete predicates and slot displays.
use super::*;
impl TryIntoApi for crate::crafting::Ingredient {
    type Api = mod_api::Ingredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::Ingredient {
            items: self.items.clone(),
            order: self.order.clone(),
            custom: self.custom.clone(),
            rule: self.rule.clone().map(Box::new),
            display: self.display.clone().map(Box::new),
            children: self
                .children
                .as_ref()
                .map(|(shape, kids)| Ok((shape.clone(), kids.try_into_api()?)))
                .transpose()?,
        })
    }
}
impl TryIntoEngine for mod_api::Ingredient {
    type Engine = crate::crafting::Ingredient;
    type Error = StackWriteError;
    fn try_into_engine(&self, _items: &Items) -> Result<Self::Engine, Self::Error> {
        let mut ingredient = crate::crafting::Ingredient {
            compiled: None,
            component_defaults: _items.component_defaults.clone(),
            shown: self
                .display
                .as_ref()
                .and_then(|display| crate::crafting::Ingredient::resolve_display(display, _items)),
            items: self.items.clone(),
            order: self.order.clone(),
            custom: self.custom.clone(),
            rule: self.rule.as_deref().cloned(),
            display: self.display.as_deref().cloned(),
            children: self
                .children
                .as_ref()
                .map(|(shape, kids)| {
                    Ok::<_, StackWriteError>((
                        shape.clone(),
                        kids.iter()
                            .map(|k| k.try_into_engine(_items))
                            .collect::<Result<_, _>>()?,
                    ))
                })
                .transpose()?,
        };
        ingredient
            .prepare_components(_items)
            .ok_or(StackWriteError::UnknownItem)?;
        Ok(ingredient)
    }
}
