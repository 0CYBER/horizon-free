//! The audience of `CommandSourceStack.broadcastToAdmins`
//! (`CommandSourceStack.java:487-497`): the players `PlayerList.isOp` names.
use horizon_client::lan::hub::Hub;

/// `PlayerList.isOp` (`PlayerList.java:587-593`): an `ops.json` entry,
/// whatever its level, or the singleplayer owner while the world allows
/// commands. On an integrated server the owner's level is 4 then and 0
/// otherwise (`IntegratedServer.getProfilePermissions`,
/// `setWorldAllowCommands`), so nobody is an operator without commands.
/// Splitscreen seats are in `ops` for their permission level (`join_split`)
/// but are no `ops.json` entries: `isOp` does not name them.
pub(super) fn is_op(hub: &Hub, uuid: u128) -> bool {
    if hub.split_players.contains(&uuid) {
        return false;
    }
    if hub.singleplayer_owner == Some(uuid) {
        return hub.ops.get(&uuid).is_some_and(|&level| level > 0);
    }
    hub.ops.contains_key(&uuid)
}

#[cfg(test)]
#[path = "admin_feedback/tests.rs"]
mod tests;
