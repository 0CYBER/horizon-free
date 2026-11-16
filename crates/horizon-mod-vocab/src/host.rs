//! The host traits, by area: one file under `host/` for each area of what the game does for a
//! mod. `Host`, `ServerHost` and `ClientHost` (and the hidden backends the handles reach) are the
//! unions of them, generated from the folder, so a new area is a new file and edits no list; a mod
//! calls through those and the handles and never names an area trait, and `prelude` names every
//! one for whoever implements them: the game, or a test double.
include!(concat!(env!("OUT_DIR"), "/mod_api_host_areas.rs"));
