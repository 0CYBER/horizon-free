use super::super::{
    ModuleCache, Source,
    tests::{assert_cached, assert_compiled, entry, fixture, replace_artifact, wasm},
};
use super::*;
use wasmtime::{Engine, Module};
use windows_sys::Win32::System::IO::DeviceIoControl;

#[test]
fn descriptor_rules() {
    let user = ProcessUser::get().unwrap();
    let sid = user.sid_string().unwrap();
    for (name, owner, dacl, trusted) in [
        ("process owner", sid.as_str(), "D:P", true),
        ("SYSTEM owner", "SY", "D:P", true),
        ("Administrators owner", "BA", "D:P", true),
        ("foreign owner", "S-1-5-21-1-2-3-9999", "D:P", false),
        ("NULL DACL", sid.as_str(), "D:NO_ACCESS_CONTROL", false),
        (
            "Users read and execute",
            sid.as_str(),
            "D:P(A;;FRFXGRGX;;;BU)",
            true,
        ),
        ("Everyone write", sid.as_str(), "D:P(A;;FW;;;WD)", false),
        (
            "Authenticated Users write",
            sid.as_str(),
            "D:P(A;;FW;;;AU)",
            false,
        ),
        ("Users write", sid.as_str(), "D:P(A;;FW;;;BU)", false),
        ("WRITE_DAC", sid.as_str(), "D:P(A;;WD;;;BU)", false),
        ("WRITE_OWNER", sid.as_str(), "D:P(A;;WO;;;BU)", false),
        ("DELETE", sid.as_str(), "D:P(A;;SD;;;BU)", false),
        ("GENERIC_ALL", sid.as_str(), "D:P(A;;GA;;;BU)", false),
        (
            "unknown access bit",
            sid.as_str(),
            "D:P(A;;0x04000000;;;BU)",
            false,
        ),
        (
            "inherit-only CREATOR OWNER",
            sid.as_str(),
            "D:P(A;OICIIO;FA;;;CO)",
            false,
        ),
        ("deny alone", sid.as_str(), "D:P(D;;GA;;;WD)", true),
        (
            "deny and allow",
            sid.as_str(),
            "D:P(D;;GA;;;WD)(A;;FW;;;WD)",
            false,
        ),
        (
            "object ACE",
            sid.as_str(),
            "D:P(OA;;FR;bf967aba-0de6-11d0-a285-00aa003049e2;;BU)",
            false,
        ),
    ] {
        let descriptor = Descriptor::from_sddl(&format!("O:{owner}{dacl}"))
            .unwrap_or_else(|why| panic!("parse {name}: {why}"));
        let result = descriptor.check(&user);
        assert_eq!(result.is_ok(), trusted, "{name}: {result:?}");
    }
}

#[test]
fn descriptor_ignores_unused_acl_storage() {
    use std::mem::MaybeUninit;

    let user = ProcessUser::get().unwrap();
    let mut storage = MaybeUninit::<[u32; 64]>::uninit();
    let acl = storage.as_mut_ptr().cast::<ACL>();
    // SAFETY: the storage is aligned and writable for the ACL's advertised capacity.
    // InitializeAcl initializes the empty ACL, whose free space need not be initialized.
    unsafe {
        assert_ne!(
            InitializeAcl(acl, size_of_val(&storage) as u32, ACL_REVISION),
            0
        );
        assert!(usize::from((*acl).AclSize) > size_of::<ACL>());
        check_descriptor(user.sid(), acl, user.sid()).unwrap();
    }
}

fn trusted_entry(cache: &ModuleCache, path: &Path) -> io::Result<File> {
    cache
        .folder
        .as_ref()
        .unwrap()
        .trusted_entry(path.file_name().unwrap())
}

fn check_private(file: &File) {
    let user = ProcessUser::get().unwrap();
    let descriptor = Descriptor::read(file).unwrap();
    descriptor.check(&user).unwrap();
    assert!(descriptor.protected().unwrap());
    // SAFETY: the descriptor and TokenUser remain live and check validated the owner.
    assert_ne!(unsafe { EqualSid(descriptor.owner, user.sid()) }, 0);
}

#[test]
fn created_folder_and_entry_are_private() {
    let (_dir, cache, engine, bytes) = fixture();
    let folder = open_directory(&cache.root, READ_CONTROL | FILE_READ_ATTRIBUTES).unwrap();
    check_private(&folder);
    assert_compiled(&cache, &engine, &bytes);
    check_private(&trusted_entry(&cache, &entry(&cache, &engine, &bytes)).unwrap());
    assert_cached(&ModuleCache::new(cache.root.clone()), &engine, &bytes);
}

fn wrong_artifact(engine: &Engine) -> Vec<u8> {
    Module::new(engine, wasm(99)).unwrap().serialize().unwrap()
}

fn everyone_writable() -> Descriptor {
    let sid = ProcessUser::get().unwrap().sid_string().unwrap();
    Descriptor::from_sddl(&format!(
        "O:{sid}D:P(A;;FA;;;{sid})(A;;FA;;;SY)(A;;FA;;;BA)(A;;FW;;;WD)"
    ))
    .unwrap()
}

#[test]
fn everyone_writable_folder_disables_cache() {
    let (_dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    let artifact = wrong_artifact(&engine);
    replace_artifact(&cache, &path, &artifact);
    // The planted file itself passes all entry checks. Folder refusal must stop the load.
    drop(trusted_entry(&cache, &path).unwrap());
    let folder = open_directory(&cache.root, WRITE_DAC | READ_CONTROL).unwrap();
    everyone_writable()
        .apply_dacl(&folder, UNPROTECTED_DACL_SECURITY_INFORMATION)
        .unwrap();
    assert!(!Descriptor::read(&folder).unwrap().protected().unwrap());
    let disabled = ModuleCache::new(cache.root.clone());
    assert!(
        disabled
            .disabled
            .as_ref()
            .unwrap()
            .contains("DACL grants unsafe access"),
        "{:?}",
        disabled.disabled
    );
    assert!(
        Descriptor::read(&folder)
            .unwrap()
            .check(&ProcessUser::get().unwrap())
            .is_err()
    );
    for _ in 0..2 {
        assert_compiled(&disabled, &engine, &bytes);
    }
    assert_eq!(fs::read(path).unwrap(), artifact);
}

#[test]
fn writable_entry_is_replaced_with_private_dacl() {
    let (_dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    let mut file = cache
        .folder
        .as_ref()
        .unwrap()
        .open(
            path.file_name().unwrap(),
            GENERIC_WRITE,
            0,
            FILE_CREATE,
            Some(&everyone_writable()),
        )
        .unwrap();
    file.write_all(&wrong_artifact(&engine)).unwrap();
    drop(file);
    assert!(
        trusted_entry(&cache, &path)
            .unwrap_err()
            .to_string()
            .contains("DACL grants unsafe access")
    );
    assert_compiled(&cache, &engine, &bytes);
    check_private(&trusted_entry(&cache, &path).unwrap());
    assert_cached(&cache, &engine, &bytes);
}

fn create_junction(path: &Path, target: &Path) {
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00a4;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xa000_0003;

    let target = target.canonicalize().unwrap();
    let target: Vec<_> = target.as_os_str().encode_wide().collect();
    let verbatim = [b'\\', b'\\', b'?', b'\\'].map(u16::from);
    let print_name = target.strip_prefix(&verbatim).unwrap();
    assert_eq!(print_name.get(1), Some(&u16::from(b':')));
    let substitute_name: Vec<_> = OsStr::new(r"\??\")
        .encode_wide()
        .chain(print_name.iter().copied())
        .collect();
    let substitute_len = u16::try_from(std::mem::size_of_val(substitute_name.as_slice())).unwrap();
    let print_len = u16::try_from(std::mem::size_of_val(print_name)).unwrap();
    let print_offset = substitute_len + 2;
    let data_len = 8 + print_offset + print_len + 2;

    // REPARSE_DATA_BUFFER with byte offsets into its two NUL-terminated names.
    let mut buffer = Vec::with_capacity(8 + usize::from(data_len));
    buffer.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    for word in [data_len, 0, 0, substitute_len, print_offset, print_len] {
        buffer.extend_from_slice(&word.to_le_bytes());
    }
    for word in substitute_name
        .into_iter()
        .chain([0])
        .chain(print_name.iter().copied())
        .chain([0])
    {
        buffer.extend_from_slice(&word.to_le_bytes());
    }
    fs::create_dir_all(path).unwrap();
    let directory = open_directory(path, GENERIC_WRITE).unwrap();
    let mut returned = 0;
    // SAFETY: the synchronous directory handle and complete input buffer remain live.
    // The operation has no output buffer or overlapped state.
    let created = unsafe {
        DeviceIoControl(
            directory.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            null_mut(),
            0,
            &mut returned,
            null_mut(),
        )
    };
    assert_ne!(
        created,
        0,
        "creating junction: {}",
        io::Error::last_os_error()
    );
}

#[test]
fn junction_root_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(super::super::CACHE_DIR);
    let target = dir.path().join("target");
    prepare_folder(&target).unwrap();
    fs::write(target.join("marker"), b"junction target").unwrap();
    create_junction(&root, &target);
    assert_eq!(fs::read(root.join("marker")).unwrap(), b"junction target");
    let cache = ModuleCache::new(root);
    assert!(
        cache.disabled.as_ref().unwrap().contains("reparse point"),
        "{:?}",
        cache.disabled
    );
}

#[test]
fn symlink_entry_is_not_followed() {
    let (_dir, cache, engine, bytes) = fixture();
    let target = cache.root.join("target");
    let artifact = wrong_artifact(&engine);
    replace_artifact(&cache, &target, &artifact);
    let path = entry(&cache, &engine, &bytes);
    match std::os::windows::fs::symlink_file(&target, &path) {
        Ok(()) => {}
        Err(why)
            if why.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD as i32)
                && std::env::var_os("CI").is_none() =>
        {
            eprintln!("symlink test skipped: {why}");
            return;
        }
        Err(why) => panic!("create symlink: {why}"),
    }
    assert!(
        trusted_entry(&cache, &path)
            .unwrap_err()
            .to_string()
            .contains("reparse point")
    );
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(fs::read(&target).unwrap(), artifact);
    assert_cached(&cache, &engine, &bytes);
}

fn make_read_only(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn read_only_corrupt_entry_is_replaced() {
    let (_dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    replace_artifact(&cache, &path, b"corrupt");
    make_read_only(&path);

    assert_compiled(&cache, &engine, &bytes);
    assert!(!fs::metadata(&path).unwrap().permissions().readonly());
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn startup_cleanup_counts_uppercase_extensions_and_removes_read_only_artifacts() {
    let (_dir, cache, engine, bytes) = fixture();
    let obsolete = cache.root.join("obsolete.CWASM");
    let current = entry(&cache, &engine, &bytes).with_extension("CwAsM");
    fs::write(&obsolete, b"old").unwrap();
    make_read_only(&obsolete);
    replace_artifact(&cache, &current, b"current");

    drop(ModuleCache::new(cache.root.clone()));
    assert!(!obsolete.exists());
    assert!(current.is_file());
    drop(ModuleCache::new_with_limit(cache.root.clone(), 0));
    assert!(!current.exists());
}

#[test]
fn hard_linked_entry_is_not_read() {
    let (dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    let alias = dir.path().join("alias");
    let artifact = wrong_artifact(&engine);
    replace_artifact(&cache, &path, &artifact);
    fs::hard_link(&path, &alias).unwrap();
    assert!(
        trusted_entry(&cache, &path)
            .unwrap_err()
            .to_string()
            .contains("multiple hard links")
    );
    assert_compiled(&cache, &engine, &bytes);
    assert_eq!(fs::read(&alias).unwrap(), artifact);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn unprotected_inherited_folder_is_upgraded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(super::super::CACHE_DIR);
    prepare_folder(root.parent().unwrap()).unwrap();
    fs::create_dir(&root).unwrap();
    let folder = open_directory(&root, READ_CONTROL | FILE_READ_ATTRIBUTES).unwrap();
    assert!(!Descriptor::read(&folder).unwrap().protected().unwrap());

    let cache = ModuleCache::new(root);
    assert_eq!(cache.disabled, None);
    let descriptor = Descriptor::read(&folder).unwrap();
    descriptor.check(&ProcessUser::get().unwrap()).unwrap();
    assert!(descriptor.protected().unwrap());
    let engine = crate::runtime::engine().unwrap();
    assert_compiled(&cache, &engine, &wasm(42));
    assert_cached(&cache, &engine, &wasm(42));
}

#[test]
fn unprotected_entry_is_replaced() {
    let (_dir, cache, engine, bytes) = fixture();
    let path = entry(&cache, &engine, &bytes);
    fs::write(&path, wrong_artifact(&engine)).unwrap();
    assert!(
        trusted_entry(&cache, &path)
            .unwrap_err()
            .to_string()
            .contains("DACL is not protected")
    );
    assert_compiled(&cache, &engine, &bytes);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn entry_with_an_open_writer_is_refused() {
    let (_dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let path = entry(&cache, &engine, &bytes);
    let writer = OpenOptions::new().write(true).open(&path).unwrap();
    assert_eq!(
        trusted_entry(&cache, &path).unwrap_err().raw_os_error(),
        Some(ERROR_SHARING_VIOLATION as i32)
    );
    drop(writer);
    assert_cached(&cache, &engine, &bytes);
}

#[test]
fn cache_operations_keep_using_the_audited_directory_after_a_rename() {
    let (dir, cache, engine, bytes) = fixture();
    assert_compiled(&cache, &engine, &bytes);
    let moved = dir.path().join("moved");
    fs::rename(&cache.root, &moved).unwrap();
    fs::create_dir(&cache.root).unwrap();
    let obsolete = "obsolete.cwasm";
    fs::write(moved.join(obsolete), b"old cache entry").unwrap();
    fs::write(cache.root.join(obsolete), b"replacement directory").unwrap();

    cache
        .folder
        .as_ref()
        .unwrap()
        .cleanup(Some(&ModuleCache::engine_part(&engine)), u64::MAX)
        .unwrap();
    assert!(!moved.join(obsolete).exists());
    assert_eq!(
        fs::read(cache.root.join(obsolete)).unwrap(),
        b"replacement directory"
    );
    assert_cached(&cache, &engine, &bytes);

    let bytes = wasm(43);
    let path = entry(&cache, &engine, &bytes);
    assert_eq!(
        cache.load_or_compile(&engine, &bytes).unwrap().1,
        Source::Compiled
    );
    assert!(moved.join(path.file_name().unwrap()).is_file());
    assert!(!path.exists());
    assert_eq!(
        cache.load_or_compile(&engine, &bytes).unwrap().1,
        Source::Cached
    );
}

#[test]
fn hidden_compiler_flag_refuses_private_windows_input() {
    let (_dir, cache, engine, bytes) = fixture();
    let input = cache.root.join(".input-1-0.tmp");
    fs::write(&input, &bytes).unwrap();
    let key = ModuleCache::key(&engine, &bytes);
    assert_eq!(
        crate::compile_child([
            OsString::from("--compile-wasm-module"),
            input.into_os_string(),
            OsString::from(&key),
        ]),
        Some(1)
    );
    assert!(!cache.entry_path(&key).exists());
    assert_eq!(
        cache.load_or_compile(&engine, &bytes).unwrap().1,
        Source::Compiled
    );
}
