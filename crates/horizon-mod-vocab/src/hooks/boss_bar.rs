//! Upstream capability retained through its existing fork handle.
pub trait Notes {
    #[notes = r###"`ServerHost::create_boss_bar(id, name)` creates a custom boss event and returns `Result<Option<BossBarRef>, TextFormError>` (`None` when the id is taken or invalid, `Err` for text deeper than 512 nodes); `ServerHost::boss_bar(id)` finds an existing one. The handle sets its name, value, maximum, colour, overlay, flags and visibility, adds or removes `PlayerRef` viewers with `add_player`/`remove_player`, and deletes the event with `remove`. These are saved custom boss events, not a per-player bar snapshot. Bundles need "hooks": @API:boss_bar@."###]
    fn boss_bar();
}
