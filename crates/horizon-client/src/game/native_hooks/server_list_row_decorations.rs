use super::*;
use crate::game::{api_values::TryIntoEngine, host::draw::GameDrawContext};
use horizon_client::multiplayer::SavedServerRow;

impl Game<'_> {
    pub(in crate::game) fn native_server_list_row_decorations(
        &self,
        call: mod_api::__private::ServerListRowDecorationsCall<'_>,
    ) {
        self.fire_server_list_row_decorations(call);
    }

    pub(in crate::game) fn decorate_server_row(
        &self,
        row: SavedServerRow<'_>,
        batch: &mut HudBatch,
    ) -> Vec<horizon_client::text::Text> {
        let first = batch.vertices.len();
        let mut tooltip = Vec::new();
        {
            let mut draw = GameDrawContext::new(self, batch);
            self.native_server_list_row_decorations(
                mod_api::__private::ServerListRowDecorationsCall {
                    name: &row.server.name,
                    address: &row.server.ip,
                    rect: row.rect,
                    visible_rect: row.visible_rect,
                    mouse: row.mouse,
                    draw: &mut draw,
                    tooltip: &mut tooltip,
                },
            );
        }
        if batch.vertices.len() > first {
            let [x, y, w, h] = row.visible_rect;
            batch.clip_since(first, x, y, x + w, y + h);
        }
        if row.mouse.is_some() {
            tooltip.try_into_engine(&()).unwrap_or_default()
        } else {
            Vec::new()
        }
    }
}
