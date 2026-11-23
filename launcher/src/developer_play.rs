//! Applying account or unsigned-developer sessions to GUI Play.
use super::*;

impl Launcher {
    pub(super) fn handle_game_session(
        &mut self,
        session: std::result::Result<Option<account::GameSession>, account::NoOwner>,
    ) {
        self.load_auth_statuses(false);
        if let Some(tag) = self.starting_client.take() {
            let instance_id = self.starting_instance.take();
            let mut cmd = backend::client_command(&tag);
            let pass = match session {
                Ok(pass) => pass,
                Err(refused) => {
                    // A keyring that didn't answer isn't a missing owner (#12179).
                    if refused.keyring.is_none() {
                        self.owner = Some(false);
                        self.owner_screen_open = true;
                    }
                    self.client_error = Some(refused.in_window());
                    self.starting_pack = None;
                    self.joining = None;
                    self.quick_play = None;
                    return;
                }
            };
            if let Some(account::GameSession { file, offline, env }) = pass {
                self.owner = Some(true);
                if let Some(file) = file {
                    cmd.arg("--session-file").arg(file);
                }
                cmd.envs(env);
                // Still play, offline, and say why.
                if let Some(e) = offline {
                    self.client_error = Some(format!("Playing offline: {e}"));
                } else if let Some(e) = account::take_unsaved() {
                    self.client_error = Some(e);
                }
            } else {
                if let Err(error) = horizon_launcher::developer_offline::require_capability(&tag) {
                    self.client_error = Some(error);
                    self.starting_pack = None;
                    self.joining = None;
                    self.quick_play = None;
                    return;
                }
                horizon_launcher::developer_offline::mark(&mut cmd);
                self.client_error = Some(tr(horizon_launcher::developer_offline::NOTICE).into());
            }
            self.accounts = Accounts::load();
            // A pack's Play button plays that pack; Play plays the selected instance.
            let pack = self.starting_pack.take();
            let instance = match (&pack, instance_id.as_deref()) {
                (Some(work), _) => instances::pack_for_launch(work),
                (None, Some(id)) => instances::for_launch_id(id),
                (None, None) => Err(tr(
                    "This instance no longer exists. Choose another instance to play.",
                )
                .into()),
            }
            .and_then(|instance| {
                if instance.resolve_version(None)? != tag {
                    return Err(tr("The instance's version changed. Press Play again.").into());
                }
                Ok(instance)
            });
            let instance = match instance {
                Ok(instance) => instance,
                Err(e) => {
                    self.client_error = Some(e);
                    self.joining = None;
                    self.quick_play = None;
                    account::discard_unread_sessions();
                    return;
                }
            };
            let run = Some(instance.id.clone());
            // What follows the profile's own arguments, in the same order: the player's extra
            // arguments come last, to add to the launcher's or override them.
            let mut after = std::process::Command::new("");
            if let Some(code) = self.joining.take() {
                if let Err(e) = backend::join_friend(&mut after, &code) {
                    self.client_error = Some(e);
                }
            }
            if let Some((flag, value)) = self.quick_play.take() {
                after.arg(flag).arg(value);
            }
            backend::append_game_args(&mut after);
            let after: Vec<_> = after.get_args().map(ToOwned::to_owned).collect();
            // Last, around every argument: the instance's pre-launch command,
            // wrapper and variables.
            self.client_vars = instance.vars(cmd.get_program());
            let vars = self.client_vars.clone();
            let launch = instance.launch.clone();
            // The profile on the launch's thread: a pack's carried mods are read whole for it.
            let profile = move |cmd: &mut std::process::Command| {
                match &pack {
                    Some(work) => horizon_launcher::modpacks::apply_profile(cmd, work)?,
                    None => instances::apply(cmd, &instance)?,
                }
                cmd.args(after);
                cmd.envs(vars.iter().map(|(k, v)| (k, v)));
                Ok(())
            };
            self.client_build = Some(tag.clone());
            self.starting_client = Some(tag);
            // The jar phase keeps the row of the started instance on Starting.
            self.starting_instance = run.clone();
            let stop = self.pre_launch_stop_for(&launch);
            jar_launch::client(cmd, profile, launch, stop, run, self.tx.clone());
        }
    }
}
