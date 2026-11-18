//! Chat commands: the logic of the commands moved out of `Game`, each over the small trait
//! it needs from the game (see [`host`]).
pub mod data;
pub mod entity;
pub mod fillbiome;
pub mod forceload;
pub mod gamerule;
pub mod help;
pub mod host;
pub mod misc;
pub mod profiler;
pub mod profiles;
pub mod publish;
pub mod recipe;
pub mod rotate;
pub mod sound;
pub mod swing;
pub mod tag;
pub mod time;
pub mod version;
pub mod weather;
pub mod worldborder;
