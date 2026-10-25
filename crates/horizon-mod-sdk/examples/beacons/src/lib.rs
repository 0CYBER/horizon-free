//! Example Horizon server mod (`docs/mods.md`, "Server mods") with
//! capability `world`: `/beacon` lights a pillar of glowstone two blocks east
//! of the player, `/ground` reads the block under their feet and the biome.
use horizon_mod_sdk::server::{BlockChange, Player, Registration, ServerMod, players, world};

/// How tall `/beacon` builds.
const HEIGHT: i32 = 4;

struct Beacons;

/// The block the player stands in: their feet, floored.
fn feet(player: &Player) -> (i32, i32, i32) {
    (
        player.x.floor() as i32,
        player.y.floor() as i32,
        player.z.floor() as i32,
    )
}

/// The pillar beside `player`: `HEIGHT` blocks of glowstone, from their feet up.
fn pillar(player: &Player) -> Vec<BlockChange> {
    let (x, y, z) = feet(player);
    (0..HEIGHT)
        .map(|dy| BlockChange {
            x: x + 2,
            y: y + dy,
            z,
            state: "minecraft:glowstone".into(),
        })
        .collect()
}

impl ServerMod for Beacons {
    fn init() -> Result<(Self, Registration), String> {
        Ok((
            Beacons,
            Registration::new().command("beacon").command("ground"),
        ))
    }

    fn command(
        &mut self,
        player: &Player,
        name: &str,
        _args: &str,
    ) -> Result<Option<String>, String> {
        let (x, y, z) = feet(player);
        if name == "beacon" {
            world::set_blocks(&player.dimension, &pillar(player))?;
            players::broadcast(&format!(
                "{} lit a beacon at {} {y} {z}",
                player.name,
                x + 2
            ));
            return Ok(None);
        }
        let below = world::get_block(&player.dimension, x, y - 1, z)
            .ok_or("The ground under you isn't loaded")?;
        let biome = world::get_biome(&player.dimension, x, y, z).unwrap_or_default();
        Ok(Some(format!("You are standing on {below} in {biome}")))
    }
}

horizon_mod_sdk::export_server_mod!(Beacons);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pillar_stands_two_blocks_east() {
        let p = Player {
            id: String::new(),
            name: "Alex".into(),
            x: -0.5,
            y: 64.0,
            z: 3.2,
            dimension: "minecraft:overworld".into(),
            game_mode: horizon_mod_sdk::server::GameMode::Survival,
            health: 20.0,
        };
        let blocks = pillar(&p);
        assert_eq!(blocks.len(), HEIGHT as usize);
        assert_eq!((blocks[0].x, blocks[0].y, blocks[0].z), (1, 64, 3));
        assert_eq!(blocks[3].y, 67);
    }
}
