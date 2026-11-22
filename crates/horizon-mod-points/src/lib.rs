//! Empty. The game's injection points were removed (#19576), and with them this crate's records
//! and listener slots. The SDK still ships its library because launchers released before then
//! pass it to every native mod build (`--extern mod_points`); delete the crate once the oldest
//! supported launcher no longer does.
