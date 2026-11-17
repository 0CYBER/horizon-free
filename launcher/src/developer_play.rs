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
            let (profile, run) = match self.starting_pack.take() {
                Some(work) => {
                    let id = instances::list()
                        .into_iter()
                        .find(|i| i.work == work)
                        .map(|i| i.id);
                    let applied = horizon_launcher::modpacks::apply_profile(&mut cmd, &work);
                    (applied, id)
                }
                None => {
                    let Some(instance) = instances::list()
                        .into_iter()
                        .find(|i| Some(&i.id) == instance_id.as_ref())
                    else {
                        self.client_error = Some(
                            tr("This instance no longer exists. Choose another instance to play.")
                                .into(),
                        );
                        self.joining = None;
                        self.quick_play = None;
                        account::discard_unread_sessions();
                        return;
                    };
                    (instances::apply(&mut cmd, &instance), Some(instance.id))
                }
            };
            if let Err(e) = profile {
                self.client_error = Some(e);
                account::discard_unread_sessions();
                return;
            }
            if let Some(code) = self.joining.take() {
                if let Err(e) = backend::join_friend(&mut cmd, &code) {
                    self.client_error = Some(e);
                }
            }
            if let Some((flag, value)) = self.quick_play.take() {
                cmd.arg(flag).arg(value);
            }
            backend::append_game_args(&mut cmd);
            // Last, around every argument: the instance's pre-launch command,
            // wrapper and variables.
            let instance = run
                .as_ref()
                .and_then(|id| instances::list().into_iter().find(|i| &i.id == id));
            self.client_vars = instance
                .as_ref()
                .map(|i| i.vars(cmd.get_program()))
                .unwrap_or_default();
            cmd.envs(self.client_vars.iter().map(|(k, v)| (k, v)));
            let launch = instance.map(|i| i.launch).unwrap_or_default();
            self.starting_client = Some(tag);
            // The jar phase keeps the row of the started instance on Starting.
            self.starting_instance = run.clone();
            jar_launch::client(cmd, launch, run, self.tx.clone());
        }
    }
}
