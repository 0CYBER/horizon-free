//! Example Horizon server mod (`docs/mods.md`, "Server mods") with
//! capability `world` and typed commands: `/beacon [height]` lights a
//! pillar of glowstone two blocks east of the player, `/ground [pos]` reads
//! the block under their feet (or at `pos`) and its biome.
use horizon_mod_sdk::server::{
    Args, Argument, BlockChange, Player, Registration, ServerMod, players, world,
};

/// How tall `/beacon` builds when no height is given.
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

/// The pillar beside `player`: `height` blocks of glowstone, from their feet up.
fn pillar(player: &Player, height: i32) -> Vec<BlockChange> {
    let (x, y, z) = feet(player);
    (0..height)
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
            Registration::new()
                .command_with(
                    "beacon",
                    [Argument::integer("height", Some(1), Some(64)).optional()],
                )
                .command_with("ground", [Argument::block_pos("pos").optional()]),
        ))
    }

    fn command(
        &mut self,
        player: &Player,
        name: &str,
        args: &Args,
    ) -> Result<Option<String>, String> {
        let (x, y, z) = feet(player);
        if name == "beacon" {
            let height = args.integer("height").unwrap_or(HEIGHT);
            world::set_blocks(&player.dimension, &pillar(player, height))?;
            players::broadcast(&format!(
                "{} lit a beacon at {} {y} {z}",
                player.name,
                x + 2
            ));
            return Ok(None);
        }
        let (bx, by, bz) = match args.block_pos("pos") {
            Some(p) => (p.x, p.y, p.z),
            None => (x, y - 1, z),
        };
        let block =
            world::get_block(&player.dimension, bx, by, bz).ok_or("That block isn't loaded")?;
        let biome = world::get_biome(&player.dimension, bx, by, bz).unwrap_or_default();
        if args.block_pos("pos").is_some() {
            return Ok(Some(format!("{bx} {by} {bz} is {block} in {biome}")));
        }
        Ok(Some(format!("You are standing on {block} in {biome}")))
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
        let blocks = pillar(&p, HEIGHT);
        assert_eq!(blocks.len(), HEIGHT as usize);
        assert_eq!((blocks[0].x, blocks[0].y, blocks[0].z), (1, 64, 3));
        assert_eq!(blocks[3].y, 67);
    }
}
