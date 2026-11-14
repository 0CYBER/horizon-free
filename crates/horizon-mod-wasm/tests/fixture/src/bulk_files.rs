use mod_api::{ConfigEntryKind, ConfigFileError, ConfigPath, ServerHost};

/// The same consumer runs natively, through the loopback, and inside the real wasm member.
pub fn check(host: &dyn ServerHost) {
    let folder = ConfigPath::new("wasm_fixture").unwrap();
    let first = ConfigPath::new("wasm_fixture/a.bin").unwrap();
    let second = ConfigPath::new("wasm_fixture/b.bin").unwrap();
    let roots = [
        host.config_files(),
        host.data_files(),
        host.world_data_files(),
    ];
    // Write every root before reading any: a collapsed selector overwrites another root.
    for (index, files) in roots.iter().enumerate() {
        files.write(&first, &[index as u8; 3][..index + 1]).unwrap();
        files.write(&second, &[index as u8 + 3]).unwrap();
    }
    for (index, files) in roots.iter().enumerate() {
        assert_eq!(files.read(&first).unwrap(), &[index as u8; 3][..index + 1]);
        let stat = files.stat(&first).unwrap().unwrap();
        assert_eq!(stat.kind, ConfigEntryKind::File);
        assert_eq!(stat.len, (index + 1) as u64);
        let page = files.list(&folder, None, 1).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].name, "a.bin");
        assert_eq!(page[0].stat.len, (index + 1) as u64);
        let next = files.list(&folder, Some(&page[0].name), 1).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].name, "b.bin");
        assert!(
            files
                .list(&folder, Some(&next[0].name), 1)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            files.list(&folder, None, 0),
            Err(ConfigFileError::InvalidPage)
        );
        assert_eq!(
            files.list(&folder, None, mod_api::CONFIG_LIST_MAX as u32 + 1),
            Err(ConfigFileError::InvalidPage)
        );
    }
    // Root paths must survive the checked wire decoder, not only named folders.
    assert!(host.data_files().list(&ConfigPath::root(), None, 1).is_ok());
    assert!(
        host.world_data_files()
            .list(&ConfigPath::root(), None, 1)
            .is_ok()
    );
    assert_eq!(
        ConfigPath::new("../escape"),
        Err(ConfigFileError::InvalidPath)
    );
    assert_eq!(
        ConfigPath::new("folder\\escape"),
        Err(ConfigFileError::InvalidPath)
    );
}

pub fn no_world(host: &dyn ServerHost) {
    let files = host.world_data_files();
    let path = ConfigPath::new("wasm_fixture/a.bin").unwrap();
    assert_eq!(files.read(&path), Err(ConfigFileError::NoWorld));
    assert_eq!(
        files.write(&path, b"refused"),
        Err(ConfigFileError::NoWorld)
    );
    assert_eq!(
        files.list(&ConfigPath::root(), None, 1),
        Err(ConfigFileError::NoWorld)
    );
    assert_eq!(files.stat(&path), Err(ConfigFileError::NoWorld));
}
