// exports: none (reached inside the crate only)
//! What a mod's parts and payloads carry that a joining client must have: the engine refuses a
//! `side = "server"` package that has any of it.
use super::*;
use crate::PayloadDirection;

impl Installed {
    /// The first part (in `owner`'s registration order) that `owner` attached and that subscribes
    /// to a client-side hook: the part's kind and the hook's name.
    pub(crate) fn client_part(&self, owner: &str) -> Option<(&'static str, &'static str)> {
        fn mine<'a, P: ?Sized + 'a>(
            owner: &'a str,
            parts: impl Iterator<Item = &'a Attached<P>> + 'a,
            subscribed: impl Fn(&P) -> &'static [Option<HookId>] + 'a,
        ) -> impl Iterator<Item = (usize, &'static str, &'static [Option<HookId>])> + 'a {
            parts
                .filter(move |attached| attached.mod_id == owner)
                .map(move |attached| {
                    let hooks = subscribed(&*attached.part);
                    (attached.order, attached.type_name, hooks)
                })
        }
        let (keys, tags) = (&self.keys, &self.tags);
        let blocks = keys.blocks.values().flatten();
        let tagged_blocks = tags.blocks.iter().map(|(_, attached)| attached);
        let items = keys.items.values().flatten();
        let tagged_items = tags.items.iter().map(|(_, attached)| attached);
        let subscriptions = mine(owner, blocks.chain(tagged_blocks), |p| p.block_subscribed())
            .chain(mine(owner, keys.block_entities.values().flatten(), |p| {
                p.block_entity_subscribed()
            }))
            .chain(mine(owner, items.chain(tagged_items), |p| {
                p.item_subscribed()
            }))
            .chain(mine(owner, keys.menus.values().flatten(), |p| {
                p.menu_subscribed()
            }))
            .chain(mine(owner, keys.fluids.values().flatten(), |p| {
                p.fluid_subscribed()
            }))
            .chain(mine(owner, keys.entities.values().flatten(), |p| {
                p.entity_subscribed()
            }));
        subscriptions
            .filter_map(|(order, kind, hooks)| {
                crate::runtime::client_hook_name(hooks).map(|hook| (order, kind, hook))
            })
            .min_by_key(|(order, ..)| *order)
            .map(|(_, kind, hook)| (kind, hook))
    }
}

impl Payloads {
    /// The first required channel `owner` declared that the server sends to clients, in
    /// channel order: a client without the mod would be refused at join for it.
    pub(crate) fn client_bound(&self, owner: &str) -> Option<&str> {
        self.phases
            .iter()
            .flat_map(|phase| phase.iter())
            .filter(|(_, entry)| {
                let declaration = &entry.declaration;
                entry.owner == owner
                    && !declaration.optional
                    && declaration.direction != PayloadDirection::ToServer
            })
            .map(|(channel, _)| channel.as_str())
            .min()
    }
}
