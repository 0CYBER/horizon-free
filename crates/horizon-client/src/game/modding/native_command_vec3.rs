//! A native command's coordinate argument: a `block_pos` (the block it names) or a `vec3`
//! (the point, `Vec3Argument.vec3()`), by the kind of the node that parsed it.
use super::Game;
use horizon_client::command::{ArgType, NodeKind, args::Coordinates};
use mod_api::CommandArgumentValue as Value;

impl Game<'_> {
    /// `coordinates` resolved against the command's source, as the `vec3` node that parsed
    /// them (`nodes` is the parse's, `range` the argument's) delivers them, else floored to the
    /// block as a `block_pos` does.
    pub(super) fn native_coords_value(
        &self,
        coordinates: &Coordinates,
        nodes: &[(usize, (usize, usize))],
        range: (usize, usize),
    ) -> Value {
        let vec3 = nodes.iter().any(|(node, at)| {
            *at == range
                && matches!(
                    &self.ui.dispatcher.0.nodes[*node].kind,
                    NodeKind::Argument {
                        ty: ArgType::Vec3 { .. },
                        ..
                    }
                )
        });
        let at = self.resolve_coords(coordinates);
        if vec3 {
            Value::Vec3(at)
        } else {
            Value::BlockPos(at.map(|v| v.floor() as i32))
        }
    }
}
