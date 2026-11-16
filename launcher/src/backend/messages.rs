use super::*;

pub enum Msg {
    Playing(Option<u64>),
    OfficialPlayers(Option<u64>),
    Storage(Result<Box<crate::storage::Report>, String>),
    Releases(Result<Vec<Release>, String>),
    Links(Links),
    Progress(String, f32),
    Installed(String, Result<(), String>),
    /// A version's files are gone (or could not be deleted).
    Removed(String, Result<(), String>),
    /// `prune_old_versions` ended; these versions were deleted.
    Pruned(Vec<String>),
    /// What each installed version takes on disk (`disk_use`).
    DiskUse(Vec<(String, u64)>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
    /// Boxed: two releases, the largest message by far.
    Updates(Result<Box<Updates>, String>),
    LauncherProgress(f32),
    LauncherUpdated(Result<Option<Staged>, String>),
    /// `reuse_prepared_everywhere` prepared these versions.
    Reused(Vec<(String, String)>),
    /// None when secrets are kept in the OS keychain, else what to say: they last only until
    /// the launcher closes, and why.
    Keychain(Option<String>),
    /// Whether an account in the launcher owned the game at its last check (the owner gate).
    Owner(bool),
    /// Credential-free result from a background account operation.
    AccountStatus(String, u64, crate::account::session::Status),
    AccountProgress(String, u64, crate::account::session::Phase),
    AccountHead(String, Option<String>, Option<Vec<[u8; 4]>>, bool),
    AccountRemoved(String, u64, Result<bool, String>),
    /// The owner gate's answer for the game about to start: the selected account's session
    /// file (none = offline) and why it plays offline, if it does.
    Session(Result<Option<crate::account::GameSession>, crate::account::NoOwner>),
    JarConsent {
        jars: Vec<horizon_platform::mod_package::manifest::JarRef>,
        reply: std::sync::mpsc::Sender<bool>,
    },
    JarClient {
        run: Option<String>,
        prepared: Box<Result<Command, String>>,
    },
    /// A modpack step's line (`modpacks`).
    PackLine(String),
    /// A modpack step ended: the pack as it now is.
    PackDone(Result<Box<crate::modpacks::Imported>, String>),
    /// The modpacks other launchers keep on this computer (`modpacks::found_on_this_computer`).
    FoundPacks(Vec<crate::other_launchers::Found>),
    /// Where the native-mods build is (`native_mods::Step::label`), and how far along.
    NativeProgress(String, crate::native_mods::Progress),
    NativeRegistryConsent(
        crate::native_mods::RegistryInventory,
        std::sync::mpsc::Sender<bool>,
    ),
    /// The native-mods build ended: the version built, or why not.
    Native(Result<String, crate::native_mods::Failed>),
    /// An imported game build is installed (`build_import::import`): its version, or why not.
    Imported(Result<String, String>),
    /// A Java instance's game is ready to start (`java_game`, wrapped): the instance's id and
    /// game folder, and the command or why not.
    /// A Java instance's game ready to start, and its variables for the post-exit command.
    JavaGame(
        String,
        PathBuf,
        Result<(Command, crate::launch::Vars), String>,
    ),
}
