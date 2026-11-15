//! Command refresh guidance carried from upstream; the command service owns the method.

pub trait Notes {
    #[notes = r#"`host.commands().refresh_commands()`, on the server's `Commands` service, asks every mod's `register_commands` again at the start of the next server tick, rebuilds the commands players may run, and sends every player on the server their tree again after `player_commands_sent`, as vanilla's `PlayerList.sendPlayerPermissionLevel` (`Commands.sendCommands`) does after `/reload`. Nothing happens where no world is open to players. Bundles need "hooks": @API:refresh_commands@.

It rebuilds the typed command dispatcher as well as the players' command trees."#]
    fn refresh_commands();
}
