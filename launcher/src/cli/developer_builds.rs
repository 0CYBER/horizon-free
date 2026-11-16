//! Explicit player consent for local unsigned game commands.
use horizon_launcher::{
    backend, build_import, developer_install,
    i18n::{tr, trf},
};
use std::path::{Path, PathBuf};

/// Runs the checkout's build code through cargo run -p xtask -- package.
/// Requires companion binaries and compiled exporters. Commit tracked changes and rebuild first.
/// Native builds use this local commit's source snapshot, including unpublished commits.
/// Select this base explicitly with settings selected-version.
#[derive(clap::Args)]
pub(super) struct DeveloperInstall {
    #[arg(help = tr("The trusted clean Horizon checkout to package"))]
    checkout: PathBuf,
    #[arg(long, help = tr("The real horizon-client executable built from that committed checkout"))]
    program: PathBuf,
    #[arg(long, required = true, help = tr("Allow executing this checkout's build code and unsigned game with your rights"))]
    trust_unsigned: bool,
}

pub(super) fn install(args: DeveloperInstall, as_json: bool) -> Result<(), String> {
    let tag = developer_install::install(&args.checkout, &args.program, args.trust_unsigned)?;
    let label = developer_install::label(&tag).unwrap();
    if as_json {
        super::json(serde_json::json!({"version": tag, "label": label}));
    } else {
        println!("{label}");
    }
    Ok(())
}

pub(super) fn import(
    path: &Path,
    version: Option<&str>,
    trust_unsigned: bool,
    as_json: bool,
) -> Result<(), String> {
    if !trust_unsigned {
        return Err(tr("An imported build is unsigned: Horizon can't check where it came from. If you trust it, pass --trust-unsigned.").into());
    }
    let tag = build_import::import(path, version)?;
    let prepared = backend::check_prepared(&tag);
    if as_json {
        super::json(serde_json::json!({ "imported": tag, "prepared": prepared }));
    } else if prepared {
        println!("{}", trf("Imported {version} and selected it. It is prepared already, from another installed version.", &[("version", &tag)]));
    } else {
        println!("{}", trf("Imported {version} and selected it. Prepare it next: horizon-launcher prepare --version {version}", &[("version", &tag)]));
    }
    Ok(())
}
