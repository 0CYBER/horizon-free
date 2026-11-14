//! The game's side of the `services` area (`horizon-mod-api/src/host/services.rs`).
use crate::game::{Game, api_values::TryIntoEngine};
use mod_api::{prelude::*, *};

impl ClientSession for Game<'_> {
    fn client_server(&self) -> Option<ClientServer> {
        mod_api::__private::check_access(
            "client_server",
            mod_api::__private::LoadAccess::ClientSetup,
        );
        self.native_client_server()
    }
    fn client_language(&self) -> Option<String> {
        mod_api::__private::check_access(
            "client_language",
            mod_api::__private::LoadAccess::ClientSetup,
        );
        self.native_client_language()
    }
    fn client_latency(&self) -> Option<u32> {
        mod_api::__private::check_access(
            "client_latency",
            mod_api::__private::LoadAccess::ClientSetup,
        );
        self.native_client_latency()
    }
    fn client_frame_stats(&self) -> Option<ClientFrameStats> {
        mod_api::__private::check_access(
            "client_frame_stats",
            mod_api::__private::LoadAccess::ClientSetup,
        );
        self.native_client_frame_stats()
    }
}
impl Commands for Game<'_> {
    fn refresh_commands(&mut self) {
        mod_api::__private::check_access(
            "refresh_commands",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_refresh_commands()
    }
    fn feedback(&mut self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        mod_api::__private::check_access("feedback", mod_api::__private::LoadAccess::WorldOnly);
        self.native_feedback(line.try_into_engine(&())?);
        Ok(())
    }
    fn run_command(&mut self, line: &str) -> bool {
        mod_api::__private::check_access("run_command", mod_api::__private::LoadAccess::WorldOnly);
        self.native_run_command(line)
    }
}
impl Levels for Game<'_> {
    fn list_levels(&self) -> Vec<LevelRef> {
        mod_api::__private::check_access("list_levels", mod_api::__private::LoadAccess::WorldOnly);
        self.native_list_levels()
    }
}
impl Messages for Game<'_> {
    fn text_allowed(&self, text: &str) -> Option<bool> {
        mod_api::__private::check_access("text_allowed", mod_api::__private::LoadAccess::WorldOnly);
        self.native_text_allowed(text)
    }
    fn broadcast(&self, line: Text) -> Result<(), mod_api::text::TextFormError> {
        mod_api::__private::check_access("broadcast", mod_api::__private::LoadAccess::WorldOnly);
        self.native_broadcast(line.try_into_engine(&())?);
        Ok(())
    }
}
impl Players for Game<'_> {
    fn server_players(&self) -> Vec<PlayerRef> {
        mod_api::__private::check_access(
            "server_players",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_server_players()
    }
    fn player_by_name(&self, name: &str) -> Option<PlayerRef> {
        mod_api::__private::check_access(
            "player_by_name",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_player_by_name(name)
    }
    fn acting_player(&self) -> Option<PlayerRef> {
        mod_api::__private::check_access(
            "acting_player",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_acting_player()
    }
}
impl RuntimeLevels for Game<'_> {
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        mod_api::__private::check_access("open", mod_api::__private::LoadAccess::WorldOnly);
        self.runtime_level_open(id, level)
    }
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        mod_api::__private::check_access("unload", mod_api::__private::LoadAccess::WorldOnly);
        self.runtime_level_unload(id)
    }
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        mod_api::__private::check_access("delete", mod_api::__private::LoadAccess::WorldOnly);
        self.runtime_level_delete(id)
    }
    fn is_open(&mut self, id: &str) -> bool {
        mod_api::__private::check_access("is_open", mod_api::__private::LoadAccess::WorldOnly);
        self.runtime_level_is_open(id)
    }
}
impl RenderAssets for Game<'_> {
    fn model(&self, id: &Identifier) -> Result<ModelRef, ModelError> {
        mod_api::__private::check_access(
            "model",
            mod_api::__private::LoadAccess::PublishedResources,
        );
        self.native_model(id)
    }
    fn render_pipeline(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        mod_api::__private::check_access(
            "render_pipeline",
            mod_api::__private::LoadAccess::PublishedResources,
        );
        self.pack.render.render_pipeline(id)
    }
    fn animation(&self, id: &Identifier) -> Option<AnimationHolder> {
        mod_api::__private::check_access(
            "animation",
            mod_api::__private::LoadAccess::PublishedResources,
        );
        self.pack.render.animation(id)
    }
}
