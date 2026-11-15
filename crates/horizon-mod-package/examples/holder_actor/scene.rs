use horizon_mod_package::{
    holder::{self, Prepared, Program, Started},
    local_control::Marker,
    selection::ModSelectionContext,
};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn until(mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < end, "controlled actor timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub fn start(root: &Path, mode: &str) -> Started {
    fs::create_dir(root.join("image")).unwrap();
    fs::create_dir(root.join("world")).unwrap();
    fs::write(root.join("image/.installed"), b"").unwrap();
    let source = std::env::current_exe().unwrap();
    let image = root.join("image").join(source.file_name().unwrap());
    fs::copy(source, &image).unwrap();
    let mut program = Program {
        executable: image.clone().into_os_string(),
        arguments: vec![],
        environment: vec![
            ("HOLDER_TEST_MODE".into(), mode.into()),
            ("HORIZON_HOME".into(), root.join("world").into_os_string()),
            ("READY".into(), root.join("ready").into_os_string()),
            ("STOP".into(), root.join("stop").into_os_string()),
        ],
        directory: root.join("world"),
        raw_windows: None,
    };
    if let Some(mask) = std::env::var_os("HOLDER_TEST_CLOSED_MASK") {
        program
            .environment
            .push(("HOLDER_TEST_CLOSED_MASK".into(), mask));
    }
    let selection = ModSelectionContext {
        game_dir: root.join("world"),
        home: root.join("world"),
        dev_mods: vec![],
    };
    let prepared = Prepared::new(
        root.join("data"),
        vec![(root.join("image"), true), (root.join("world"), false)],
        program,
        &selection,
    )
    .unwrap();
    let marker = Marker::new(root.join("endpoint")).unwrap();
    let mut command = Command::new(image);
    command
        .env_remove("HOLDER_TEST_MODE")
        .env_remove("HOLDER_TEST_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    holder::launch(
        command,
        &marker,
        &prepared,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap()
}
