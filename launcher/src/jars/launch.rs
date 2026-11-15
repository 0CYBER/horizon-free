//! Prepare data for the final game command, before shell or graphics wrappers obscure it.
use super::{prepare, Permission, PrepareError};
use crate::{
    i18n::trf,
    launch::{self, Launch},
};
use horizon_mod_package::{
    manifest::{collect_jar_requirements, JarConflict},
    mod_list::ModList,
    packages,
    selection::{self, LaunchArguments},
};
use std::{fmt, io, path::PathBuf, process::Command};

#[derive(Debug)]
#[non_exhaustive]
pub enum LaunchError {
    Command { program: PathBuf, source: io::Error },
    Packages(packages::ReadError),
    Conflict(JarConflict),
    Prepare(PrepareError),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command { program, source } => write!(f, "{}: {source}", program.display()),
            Self::Packages(error) => fmt::Display::fmt(error, f),
            Self::Conflict(error) => f.write_str(&trf("SHA-512 {hash} names both {first_id} {first_version} and {second_id} {second_version}.", &[
                ("hash", error.first.sha512()), ("first_id", error.first.id()), ("first_version", error.first.version()),
                ("second_id", error.second.id()), ("second_version", error.second.version()),
            ])),
            Self::Prepare(error) => fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Command { source, .. } => source,
            Self::Packages(error) => error,
            Self::Conflict(error) => error,
            Self::Prepare(error) => error,
        })
    }
}

/// The launcher prepares data from manifests; compiled identity is the engine's responsibility.
/// The command must already include the instance/modpack, quick-play and global arguments.
/// Per-instance environment is applied here as well as by `launch::wrap`, so both see the same
/// effective home. No game, Java or native-source fixture is executed during preparation.
pub fn prepare_launch(
    command: &mut Command,
    launch: &Launch,
    permission: Permission<'_>,
) -> Result<(), LaunchError> {
    launch::apply_environment(command, launch);
    let command_error = |source| LaunchError::Command {
        program: command.get_program().into(),
        source,
    };
    let args = LaunchArguments::for_command(command).map_err(command_error)?;
    if args.is_information_request() {
        return Ok(());
    }
    let cwd = std::path::absolute(
        command
            .get_current_dir()
            .unwrap_or(std::path::Path::new(".")),
    )
    .map_err(command_error)?;
    let executable =
        std::fs::canonicalize(cwd.join(command.get_program())).map_err(command_error)?;
    let native = packages::adjacent_manifests(&executable).map_err(LaunchError::Packages)?;
    let state = ModList::load(&args.context.game_dir);
    let content: Vec<_> = selection::content_packages(
        &args.context.game_dir,
        &state.enabled,
        &args.context.dev_mods,
    )
    .collect();
    let jars = collect_jar_requirements(
        native
            .iter()
            .map(|p| &p.manifest)
            .chain(content.iter().map(|p| &p.package.manifest))
            .map(|m| (m.id.as_str(), m.jar.as_slice())),
    )
    .map_err(LaunchError::Conflict)?;
    prepare(&args.context.home, &jars, permission).map_err(LaunchError::Prepare)
}
