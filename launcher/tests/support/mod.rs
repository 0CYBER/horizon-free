use horizon_launcher::process;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub fn console_probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("horizon-console-probe-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("console-probe.exe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/console_probe.rs");
        let output = process::command("rustc")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        executable
    })
}
