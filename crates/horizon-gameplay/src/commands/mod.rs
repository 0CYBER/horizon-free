//! Chat commands: the logic of the commands moved out of `Game`, each over the small trait
//! it needs from the game (see [`host`]).
pub mod gamerule;
pub mod help;
pub mod host;
pub mod time;
pub mod weather;
