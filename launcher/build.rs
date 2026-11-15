//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
fn main() {
    println!("cargo:rerun-if-env-changed=HORIZON_LAUNCHER_REPOSITORY");
    println!("cargo:rustc-check-cfg=cfg(horizon_fork_launcher)");
    let repository =
        std::env::var("HORIZON_LAUNCHER_REPOSITORY").unwrap_or_else(|_| "arismoko/horizon".into());
    match repository.as_str() {
        "arismoko/horizon" => println!("cargo:rustc-cfg=horizon_fork_launcher"),
        "horizon-rust/horizon" => {}
        _ => panic!("unsupported launcher repository: {repository}"),
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Resolve imports before main from System32, never a nearby injector DLL (#13108).
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
        // Unoptimised builds overflow Windows' default 1 MiB main-thread stack while the window
        // builds its first frame (0xC00000FD at start); release builds keep the default.
        if std::env::var("PROFILE").as_deref() == Ok("debug") {
            println!("cargo:rustc-link-arg-bins=/STACK:16777216");
        }
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    // Microsoft sign-in in a local build: the fork's own Azure client ID from an untracked
    // `launcher/.msa-client-id` (never committed), when the variable isn't set. Release builds
    // set HORIZON_MSA_CLIENT_ID from the repository variable instead. Watched only once it exists
    // (a missing watched file would rerun this script on every build): after adding it, change
    // build.rs or clean once.
    let id_file = format!("{dir}/.msa-client-id");
    if std::path::Path::new(&id_file).is_file() {
        println!("cargo:rerun-if-changed={id_file}");
    }
    println!("cargo:rerun-if-env-changed=HORIZON_MSA_CLIENT_ID");
    if std::env::var_os("HORIZON_MSA_CLIENT_ID").is_none() {
        if let Ok(id) = std::fs::read_to_string(&id_file) {
            let id = id.trim();
            if !id.is_empty() {
                println!("cargo:rustc-env=HORIZON_MSA_CLIENT_ID={id}");
            }
        }
    }
    let icon = "horizon.ico";
    let ico = format!("{dir}/../packaging/icons/{icon}");
    println!("cargo:rerun-if-changed={ico}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if repository == "arismoko/horizon" {
            resource.set("FileDescription", "Horizon Launcher");
            resource.set("ProductName", "Horizon Launcher");
            resource.set("InternalName", "horizon-fork-launcher");
            resource.set("OriginalFilename", "horizon-fork-launcher.exe");
        }
        if let Err(e) = resource.compile() {
            panic!("embedding {ico}: {e}");
        }
    }
}
