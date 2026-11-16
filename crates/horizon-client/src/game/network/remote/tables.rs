//! Server gameplay item tags. Typed mod tag notices remain in remote_tags.
use super::super::Game;
use horizon_client::items::Items;
use horizon_protocol::registry::Dynamic;
use std::{collections::HashMap, sync::Arc};
impl Game<'_> {
    /// The game's items with the server's block and item tags, by name. The
    /// window's own server thread plays with the world's tables instead.
    pub(in crate::game) fn server_items(&self, registries: &Dynamic) -> Option<Arc<Items>> {
        let r = self.remote.as_ref().filter(|_| self.integrated.is_none())?;
        let names = |registry: &str| -> Option<HashMap<String, Vec<String>>> {
            let ids = r.builtin.get(registry)?;
            let tags = registries
                .tags
                .get(registry)?
                .iter()
                .map(|(name, members)| {
                    let members = members.iter().filter_map(|&id| ids.name(id));
                    (name.clone(), members.map(str::to_owned).collect())
                });
            Some(tags.collect())
        };
        let (blocks, items) = (names("minecraft:block"), names("minecraft:item"));
        if blocks.is_none() && items.is_none() {
            return None;
        }
        Some(Arc::new(self.res.items().with_tags(blocks, items)))
    }
}
#[cfg(test)]
mod tests {
    use super::super::super::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use horizon_protocol::{
        connection::Pipe,
        packets::configuration::{RegistryTags, TagEntry, UpdateTags},
    };
    use std::collections::HashSet;

    fn client() -> Game<'static> {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES")
                .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let res: &'static _ = Box::leak(Box::new(loaded.resources));
        let mut game = Game::new(
            res,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "TagTest").unwrap();
        game
    }

    /// A server whose data pack makes stone climbable and sticks planks: the
    /// client reads its tags, not the game's own (#7843).
    #[test]
    #[ignore = "daily"]
    fn a_servers_block_and_item_tags_replace_the_games() {
        let mut game = client();
        assert!(!game.items().block_tags["minecraft:climbable"].contains("minecraft:stone"));
        let builtin = game.remote.as_ref().unwrap().builtin.clone();
        let id = |registry: &str, name: &str| builtin.get(registry).unwrap().id(name).unwrap();
        let tag = |registry: &str, name: &str, members: &[&str]| RegistryTags {
            registry: registry.into(),
            tags: vec![TagEntry {
                name: name.into(),
                ids: members.iter().map(|m| id(registry, m)).collect(),
            }],
        };
        game.remote_update_tags(&UpdateTags {
            registries: vec![
                tag(
                    "minecraft:block",
                    "minecraft:climbable",
                    &["minecraft:ladder", "minecraft:stone"],
                ),
                tag(
                    "minecraft:item",
                    "minecraft:planks",
                    &["minecraft:stick", "minecraft:oak_planks"],
                ),
            ],
        });

        let items = game.items();
        let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<HashSet<_>>();
        assert_eq!(
            items.block_tags["minecraft:climbable"],
            set(&["minecraft:ladder", "minecraft:stone"])
        );
        // The server's tags replace the game's: a tag it didn't send is gone.
        assert!(!items.block_tags.contains_key("minecraft:fences"));
        assert_eq!(
            items.item_tag_order["minecraft:planks"],
            ["minecraft:stick", "minecraft:oak_planks"]
        );
        assert!(items.item_tags["minecraft:planks"].contains("minecraft:stick"));
    }
}
