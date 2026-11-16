//! Resource reads for native mods through the game's selected pack stacks.
use super::Game;
use horizon_assets::resource_source::read_winning_resource;
use mod_api::{Identifier, PackType};

impl Game<'_> {
    /// The winning bytes of `identifier` in `pack_type`'s stack. Client
    /// resources are already resolved into `PackResources::source` by
    /// `packs::resources` (filters and overrides included); server data is
    /// resolved here in vanilla's priority order: the selected world packs
    /// (later overrides earlier), then the mods' content root, then the game's
    /// own data unless `minecraft:vanilla` is disabled.
    pub(in crate::game) fn resource(
        &self,
        pack_type: PackType,
        identifier: &Identifier,
    ) -> Option<Vec<u8>> {
        let (top, sources) = if pack_type == PackType::ClientResources {
            // A server has no client resource stack to read; only a client
            // selects the asset packs this asks about.
            if !self.options.execution_role.is_client() {
                return None;
            }
            ("assets", vec![&self.pack.source])
        } else {
            let mut sources = Vec::new();
            if !self.cmds.vanilla_disabled {
                sources.push(&self.pack.data_source);
            }
            sources.push(&self.pack.data_changes);
            if self.options.execution_role.is_logical_server() {
                sources.extend(self.fns.pack_sources().rev());
            }
            ("data", sources)
        };
        let name = format!("{top}/{}/{}", identifier.namespace(), identifier.path());
        read_winning_resource(&sources, &name, mod_api::MAX_RESOURCE_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> Identifier {
        Identifier::parse(value).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn server_resource_uses_the_higher_of_two_selected_world_packs() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            game.world_root = scratch.path().to_owned();
            for (name, bytes) in [("low", "low"), ("high", "high")] {
                let pack = game.world_root.join("datapacks").join(name);
                std::fs::create_dir_all(pack.join("data/demo/value")).unwrap();
                std::fs::write(
                    pack.join("pack.mcmeta"),
                    r#"{"pack":{"description":"test","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#,
                )
                .unwrap();
                std::fs::write(pack.join("data/demo/value/order.json"), bytes).unwrap();
            }
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("reload"));
            assert!(game.run_command("datapack disable \"file/high\""));
            assert!(game.run_command("datapack disable \"file/low\""));
            assert!(game.run_command("datapack enable \"file/low\" first"));
            assert!(game.run_command("datapack enable \"file/high\" last"));
            assert_eq!(
                game.resource(PackType::ServerData, &id("demo:value/order.json"))
                    .as_deref(),
                Some(b"high".as_slice())
            );
            assert!(game.run_command("datapack disable \"file/low\""));
            assert!(game.run_command("datapack enable \"file/low\" last"));
            assert_eq!(
                game.resource(PackType::ServerData, &id("demo:value/order.json"))
                    .as_deref(),
                Some(b"low".as_slice())
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn server_data_reads_vanilla_files_and_refuses_missing_ones() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let bytes = game
                .resource(
                    PackType::ServerData,
                    &id("minecraft:enchantment/sharpness.json"),
                )
                .expect("the game's own data pack holds sharpness");
            assert!(bytes.starts_with(b"{"));
            assert!(
                game.resource(
                    PackType::ServerData,
                    &id("minecraft:enchantment/not_a_real_enchantment.json")
                )
                .is_none()
            );
            // A server has no selected client asset stack to read.
            assert!(
                game.resource(
                    PackType::ClientResources,
                    &id("minecraft:models/block/stone.json")
                )
                .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn server_data_reads_mod_content_and_lets_it_override_the_game() {
        use horizon_assets::resource_source::{Resource, ResourceSource};
        use std::collections::BTreeMap;
        super::super::native_entity_refs::tests::with_game(|game| {
            let vanilla = game
                .resource(
                    PackType::ServerData,
                    &id("minecraft:enchantment/sharpness.json"),
                )
                .expect("the game's own data pack holds sharpness");
            game.pack.data_changes = ResourceSource::indexed(BTreeMap::from([
                (
                    "data/demo/machines/furnace.json".to_owned(),
                    Resource::bytes(b"{\"mod\":true}".to_vec()),
                ),
                (
                    "data/minecraft/enchantment/sharpness.json".to_owned(),
                    Resource::bytes(b"{\"pack\":true}".to_vec()),
                ),
            ]));
            // A mod's own data file resolves through its content layer.
            assert_eq!(
                game.resource(PackType::ServerData, &id("demo:machines/furnace.json"))
                    .as_deref(),
                Some(&b"{\"mod\":true}"[..])
            );
            // A later layer in the stack overrides the game's own data.
            assert_eq!(
                game.resource(
                    PackType::ServerData,
                    &id("minecraft:enchantment/sharpness.json")
                )
                .as_deref(),
                Some(&b"{\"pack\":true}"[..])
            );
            assert_ne!(vanilla, b"{\"pack\":true}");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn client_assets_read_the_winning_pack_file() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let bytes = game
                .resource(
                    PackType::ClientResources,
                    &id("minecraft:models/block/stone.json"),
                )
                .expect("the vanilla asset pack holds stone's model");
            assert!(bytes.starts_with(b"{"));
            assert!(
                game.resource(
                    PackType::ClientResources,
                    &id("minecraft:models/block/not_a_real_model.json")
                )
                .is_none()
            );
        });
    }
}
