use super::*;

impl Hub {
    /// Publish the frozen server token inventory before sending any command trees.
    pub fn set_selector_tokens(
        &mut self,
        tokens: Vec<horizon_client_core::registries::SelectorToken>,
    ) {
        self.selector_tokens = tokens;
    }

    /// Commands.sendCommands: the command tree for this player. Public because `Hub::set_mod_commands`, a function
    /// replacement's target (`tools/ci/fn-replace-check/lower`), calls it from a copy in another crate.
    pub fn send_commands(&self, i: usize) {
        let g = &self.guests[i];
        let mut packet = self.commands_packet(g.permission);
        // What the mods took out is not in the tree the client gets.
        if !g.hidden_commands.is_empty() {
            let hidden: Vec<i32> = packet
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind == 1 && g.hidden_commands.contains(&n.name))
                .map(|(i, _)| i as i32)
                .collect();
            let root = packet.root as usize;
            if let Some(root) = packet.nodes.get_mut(root) {
                root.children.retain(|c| !hidden.contains(c));
            }
        }
        if g.brand == "horizon" {
            g.send(&C::CustomPayload(
                horizon_protocol::packets::configuration::CustomPayload {
                    channel: crate::lan::commands::selector_sync::CHANNEL.into(),
                    data: crate::lan::commands::selector_sync::encode(&self.selector_tokens),
                },
            ));
        }
        g.send(&C::Commands(packet));
    }
}
