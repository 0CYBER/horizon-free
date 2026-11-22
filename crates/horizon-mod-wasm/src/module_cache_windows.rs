use std::{
    ffi::{OsStr, OsString, c_void},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_NAMES_INFORMATION, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_IGNORE_READONLY_ATTRIBUTE,
            FILE_RENAME_INFORMATION, FILE_RENAME_REPLACE_IF_EXISTS, FILE_SYNCHRONOUS_IO_NONALERT,
            FileNamesInformation, FileRenameInformation, FileRenameInformationEx,
            NTCREATEFILE_CREATE_DISPOSITION, NtCreateFile, NtQueryDirectoryFile,
            NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{
            IO::IO_STATUS_BLOCK,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
};

use super::{STALE_TEMPORARY, current_entry, is_cache_artifact, temporary_name};

const FILE_PERSISTENT_ACLS: u32 = 0x8;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_RAMDISK: u32 = 6;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const FOREIGN_ACCESS: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | GENERIC_READ | GENERIC_EXECUTE;

struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns a Win32 LocalAlloc-family allocation.
        unsafe { LocalFree(self.0) };
    }
}

struct ProcessUser(Vec<usize>);

impl ProcessUser {
    fn get() -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: the process pseudo-handle is valid and token is a writable output.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken returned a new owned handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut size = 0;
        // SAFETY: a null buffer with length zero queries the required size.
        let result = unsafe {
            GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut size)
        };
        let error = io::Error::last_os_error();
        if result == 0 && error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }
        if (size as usize) < size_of::<TOKEN_USER>() {
            return Err(io::Error::other("invalid process token user size"));
        }
        let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        // SAFETY: the aligned buffer covers size bytes and the token remains open.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(storage))
    }

    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation initialized an aligned TOKEN_USER, including its SID.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    fn sid_string(&self) -> io::Result<String> {
        let mut string = null_mut();
        // SAFETY: the SID is live in self and string is a writable output.
        if unsafe { ConvertSidToStringSidW(self.sid(), &mut string) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _owner = Local(string.cast());
        let mut length = 0;
        // SAFETY: the returned allocation is NUL-terminated and remains owned above.
        while unsafe { *string.add(length) } != 0 {
            length += 1;
        }
        // SAFETY: length is within the allocation and excludes the terminator.
        String::from_utf16(unsafe { std::slice::from_raw_parts(string, length) })
            .map_err(|_| io::ErrorKind::InvalidData.into())
    }
}

struct Descriptor {
    storage: Local,
    owner: PSID,
    dacl: *mut ACL,
}

impl Descriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let sddl = wide(OsStr::new(sddl))?;
        let mut descriptor = null_mut();
        // SAFETY: sddl is terminated and descriptor is a writable output.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let storage = Local(descriptor);
        let (mut owner, mut dacl) = (null_mut(), null_mut());
        let (mut present, mut defaulted) = (0, 0);
        // SAFETY: the Win32 descriptor is live and the out parameters are writable.
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) } == 0
            || unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
            } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            storage,
            owner,
            dacl,
        })
    }

    fn private(user: &ProcessUser, directory: bool) -> io::Result<Self> {
        let sid = user.sid_string()?;
        let inherit = if directory { "OICI" } else { "" };
        Self::from_sddl(&format!(
            "O:{sid}D:P(A;{inherit};FA;;;{sid})(A;{inherit};FA;;;SY)(A;{inherit};FA;;;BA)"
        ))
    }

    fn read(file: &File) -> io::Result<Self> {
        let (mut owner, mut dacl, mut descriptor) = (null_mut(), null_mut(), null_mut());
        // SAFETY: the File holds the handle and the out parameters are writable. On success
        // owner and dacl point into the returned descriptor allocation.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        status_result(status)?;
        Ok(Self {
            storage: Local(descriptor),
            owner,
            dacl,
        })
    }

    fn check(&self, user: &ProcessUser) -> io::Result<()> {
        // SAFETY: both descriptor pointers are null or borrowed from self.storage.
        unsafe { check_descriptor(self.owner, self.dacl, user.sid()) }
    }

    fn protected(&self) -> io::Result<bool> {
        let (mut control, mut revision) = (0, 0);
        // SAFETY: the descriptor is live and both output slots are writable.
        if unsafe { GetSecurityDescriptorControl(self.storage.0, &mut control, &mut revision) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(control & SE_DACL_PROTECTED != 0)
    }

    fn apply_dacl(&self, file: &File, protection: u32) -> io::Result<()> {
        if self.dacl.is_null() {
            return Err(io::Error::other("refusing to set a NULL DACL"));
        }
        // SAFETY: the DACL is borrowed from our descriptor, and file owns the live handle.
        status_result(unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | protection,
                null_mut(),
                null_mut(),
                self.dacl,
                null(),
            )
        })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.storage.0,
            bInheritHandle: 0,
        }
    }
}

fn status_result(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

/// SAFETY: owner and dacl are borrowed from a Win32 descriptor, and user is a live TokenUser SID.
unsafe fn check_descriptor(owner: PSID, dacl: *const ACL, user: PSID) -> io::Result<()> {
    // SAFETY: callers keep the descriptor and token buffers live.
    if owner.is_null() || unsafe { IsValidSid(owner) == 0 || !trusted_sid(owner, user) } {
        return Err(io::Error::other("untrusted Windows owner"));
    }
    if dacl.is_null() {
        return Err(io::Error::other("NULL Windows DACL"));
    }
    let invalid_acl = || io::Error::other("invalid Windows DACL");
    // SAFETY: the DACL is supplied by Win32, and the bounds below limit all reads inside it.
    if unsafe { IsValidAcl(dacl) } == 0 {
        return Err(invalid_acl());
    }
    let mut information = ACL_SIZE_INFORMATION::default();
    // SAFETY: Win32 validated this live ACL and the output has the required layout.
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: IsValidAcl validated the ACL header.
    let acl = unsafe { &*dacl };
    let used = information.AclBytesInUse as usize;
    let mut offset = size_of::<ACL>();
    if !(offset..=usize::from(acl.AclSize)).contains(&used)
        || information.AceCount != u32::from(acl.AceCount)
    {
        return Err(invalid_acl());
    }
    // The free space up to AclSize can contain uninitialized bytes. Read only the
    // header and the fields of ACE types the trust check understands.
    for _ in 0..acl.AceCount {
        let remaining = used - offset;
        if remaining < size_of::<ACE_HEADER>() {
            return Err(invalid_acl());
        }
        // SAFETY: the header lies in the populated ACL and each ACE is DWORD-aligned.
        let ace = unsafe { dacl.cast::<u8>().add(offset) };
        let header = unsafe { ace.cast::<ACE_HEADER>().read() };
        let size = usize::from(header.AceSize);
        if !(size_of::<ACE_HEADER>()..=remaining).contains(&size)
            || !size.is_multiple_of(size_of::<u32>())
        {
            return Err(invalid_acl());
        }
        offset += size;
        match header.AceType {
            // Examine inherit-only allows as well. A deny never offsets an unsafe allow.
            ACCESS_ALLOWED_ACE_TYPE => {
                const SID_OFFSET: usize = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
                const SID_HEADER: usize = offset_of!(SID, SubAuthority);
                if size < SID_OFFSET + SID_HEADER {
                    return Err(invalid_acl());
                }
                // SAFETY: these offsets are within the bounded ACE and the SID header is present.
                let sid = unsafe { ace.add(SID_OFFSET) };
                let count = unsafe { sid.add(offset_of!(SID, SubAuthorityCount)).read() };
                if size - SID_OFFSET != SID_HEADER + usize::from(count) * size_of::<u32>() {
                    return Err(invalid_acl());
                }
                let sid = sid.cast_mut().cast();
                // SAFETY: the SID and its subauthorities fit within this ACE.
                if unsafe { IsValidSid(sid) } == 0 {
                    return Err(invalid_acl());
                }
                // SAFETY: the mask is DWORD-aligned and precedes the bounded SID.
                let mask = unsafe {
                    ace.add(offset_of!(ACCESS_ALLOWED_ACE, Mask))
                        .cast::<u32>()
                        .read()
                };
                // SAFETY: both SIDs were supplied by Win32 or validated above.
                if mask & !FOREIGN_ACCESS != 0 && !unsafe { trusted_sid(sid, user) } {
                    return Err(io::Error::other("Windows DACL grants unsafe access"));
                }
            }
            ACCESS_DENIED_ACE_TYPE => {}
            _ => return Err(io::Error::other("unsupported Windows ACE type")),
        }
    }
    if offset != used {
        return Err(invalid_acl());
    }
    Ok(())
}

/// SAFETY: both pointers are valid SIDs for the duration of the calls.
unsafe fn trusted_sid(sid: PSID, user: PSID) -> bool {
    // SAFETY: caller supplies live, valid SIDs.
    unsafe {
        EqualSid(sid, user) != 0
            || IsWellKnownSid(sid, WinLocalSystemSid) != 0
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut words: Vec<_> = value.encode_wide().collect();
    if words.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "interior NUL"));
    }
    words.push(0);
    Ok(words)
}

fn parent_and_leaf(path: &Path) -> io::Result<(&Path, &OsStr)> {
    let leaf = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok((parent, leaf))
}

fn file_information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file owns the handle and info is a writable output.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn file_identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    // ReFS needs all 128 bits of the file ID when comparing handles.
    let mut info = FILE_ID_INFO::default();
    // SAFETY: file owns the handle and info has the layout and size for FileIdInfo.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

fn check_directory(file: &File) -> io::Result<()> {
    let info = file_information(file)?;
    if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
        != FILE_ATTRIBUTE_DIRECTORY
    {
        return Err(io::Error::other(
            "expected a directory without a reparse point",
        ));
    }
    Ok(())
}

fn check_volume(file: &File) -> io::Result<()> {
    let mut flags = 0;
    // SAFETY: the handle is live and flags is a writable output. Other outputs are optional.
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            &mut flags,
            null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(io::Error::other(
            "volume does not enforce persistent Windows ACLs",
        ));
    }

    let mut name = vec![0u16; 256];
    loop {
        // SAFETY: file owns the handle and name is writable for the specified length.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                name.as_mut_ptr(),
                name.len() as u32,
                VOLUME_NAME_GUID | FILE_NAME_OPENED,
            )
        } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < name.len() {
            name.truncate(length);
            break;
        }
        name.resize(length + 1, 0);
    }
    // A volume GUID identifies the actual volume even under a mounted folder. Network
    // shares have no volume GUID. The opened name avoids normalizing path components
    // when only the volume is needed.
    let path = PathBuf::from(OsString::from_wide(&name));
    let volume = path.ancestors().last().ok_or(io::ErrorKind::InvalidData)?;
    let volume = wide(volume.as_os_str())?;
    // SAFETY: volume is the NUL-terminated root returned for the owned handle.
    if !matches!(
        unsafe { GetDriveTypeW(volume.as_ptr()) },
        DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK
    ) {
        return Err(io::Error::other("cache requires a local Windows volume"));
    }
    Ok(())
}

fn open_directory(path: &Path, access: u32) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

pub(super) struct Folder {
    file: File,
    user: ProcessUser,
}

pub(super) fn prepare_folder(root: &Path) -> io::Result<Folder> {
    let (parent, leaf) = parent_and_leaf(root)?;
    fs::create_dir_all(parent)?;
    let root = parent.canonicalize()?.join(leaf);
    let name = wide(root.as_os_str())?;
    let user = ProcessUser::get()?;
    let private = Descriptor::private(&user, true)?;
    // SAFETY: the name is NUL-terminated and private outlives its SECURITY_ATTRIBUTES.
    if unsafe { CreateDirectoryW(name.as_ptr(), &private.attributes()) } == 0 {
        let why = io::Error::last_os_error();
        if why.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(why);
        }
    }

    let access = READ_CONTROL | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_TRAVERSE;
    let folder = open_directory(&root, access)?;
    check_directory(&folder)?;
    check_volume(&folder)?;
    let descriptor = Descriptor::read(&folder)?;
    descriptor.check(&user)?;
    if !descriptor.protected()? {
        // MAXIMUM_ALLOWED suppresses SetSecurityInfo propagation to existing children.
        // Repairing a folder must not make any previously writable artifact appear trusted.
        let writable = open_directory(&root, access | WRITE_DAC | MAXIMUM_ALLOWED)?;
        check_directory(&writable)?;
        if file_identity(&writable)? != file_identity(&folder)? {
            return Err(io::Error::other(
                "cache directory changed during verification",
            ));
        }
        Descriptor::read(&writable)?.check(&user)?;
        private.apply_dacl(&writable, PROTECTED_DACL_SECURITY_INFORMATION)?;
        check_directory(&writable)?;
        check_volume(&writable)?;
        let descriptor = Descriptor::read(&writable)?;
        descriptor.check(&user)?;
        if !descriptor.protected()? {
            return Err(io::Error::other("cache directory DACL is not protected"));
        }
    }
    Ok(Folder { file: folder, user })
}

fn component(name: &OsStr) -> io::Result<Vec<u16>> {
    let name: Vec<_> = name.encode_wide().collect();
    if name.is_empty()
        || name.len() > 255
        || name == [u16::from(b'.')]
        || name == [u16::from(b'.'); 2]
        || name
            .iter()
            .any(|&word| matches!(word, 0 | 0x2f | 0x3a | 0x5c))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(name)
}

fn nt_result(status: NTSTATUS) -> io::Result<()> {
    if status == STATUS_SUCCESS {
        Ok(())
    } else {
        // SAFETY: accepts an NTSTATUS value and uses no caller memory.
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ))
    }
}

impl Folder {
    fn open(
        &self,
        name: &OsStr,
        access: u32,
        share: FILE_SHARE_MODE,
        disposition: NTCREATEFILE_CREATE_DISPOSITION,
        security: Option<&Descriptor>,
    ) -> io::Result<File> {
        let mut name = component(name)?;
        let length = (name.len() * size_of::<u16>()) as u16;
        let name = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: name.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: self.file.as_raw_handle(),
            ObjectName: &name,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: security.map_or(null(), |descriptor| descriptor.storage.0.cast()),
            SecurityQualityOfService: null(),
        };
        let mut handle = null_mut();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: the directory handle and counted single-component name are live.
        // The optional descriptor and both outputs remain live throughout the call.
        nt_result(unsafe {
            NtCreateFile(
                &mut handle,
                access | SYNCHRONIZE,
                &attributes,
                &mut status,
                null(),
                FILE_ATTRIBUTE_NORMAL,
                share,
                disposition,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
                null(),
                0,
            )
        })?;
        // SAFETY: successful NtCreateFile returned a new exclusively owned handle.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    pub(super) fn trusted_entry(&self, name: &OsStr) -> io::Result<File> {
        let file = self.open(
            name,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            FILE_OPEN,
            None,
        )?;
        let info = file_information(&file)?;
        // SAFETY: file owns the live handle.
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
            || info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
        {
            return Err(io::Error::other(
                "entry is not a regular file without a reparse point",
            ));
        }
        if info.nNumberOfLinks != 1 {
            return Err(io::Error::other("entry has multiple hard links"));
        }
        let descriptor = Descriptor::read(&file)?;
        descriptor.check(&self.user)?;
        // Inherited access may have changed since an existing file was written.
        if !descriptor.protected()? {
            return Err(io::Error::other("entry DACL is not protected"));
        }
        Ok(file)
    }

    pub(super) fn write(&self, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
        let private = Descriptor::private(&self.user, false)?;
        let mut file = loop {
            let name = temporary_name();
            match self.open(
                OsStr::new(&name),
                GENERIC_WRITE | DELETE,
                0,
                FILE_CREATE,
                Some(&private),
            ) {
                Ok(file) => break file,
                Err(why) if why.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(why) => return Err(why),
            }
        };
        let result = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| self.publish(&file, name));
        if result.is_err() {
            let _ = delete(&file, false);
        }
        result
    }

    fn publish(&self, file: &File, name: &OsStr) -> io::Result<()> {
        let name = component(name)?;
        let length = name.len() * size_of::<u16>();
        let size = size_of::<FILE_RENAME_INFORMATION>() + length;
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let buffer = storage.as_mut_ptr().cast::<u8>();
        let info = buffer.cast::<FILE_RENAME_INFORMATION>();
        // SAFETY: storage is aligned for the header and holds the entire trailing name.
        unsafe {
            (*info).Anonymous.ReplaceIfExists = true;
            (*info).RootDirectory = self.file.as_raw_handle();
            (*info).FileNameLength = length as u32;
            buffer
                .add(offset_of!(FILE_RENAME_INFORMATION, FileName))
                .cast::<u16>()
                .copy_from_nonoverlapping(name.as_ptr(), name.len());
        }
        let rename = |class| {
            let mut status = IO_STATUS_BLOCK::default();
            // SAFETY: the buffer matches both rename classes and its destination is a
            // single component under the live, verified directory handle.
            nt_result(unsafe {
                NtSetInformationFile(
                    file.as_raw_handle(),
                    &mut status,
                    buffer.cast(),
                    size as u32,
                    class,
                )
            })
        };
        let result = rename(FileRenameInformation);
        if result
            .as_ref()
            .is_err_and(|why| why.kind() == io::ErrorKind::PermissionDenied)
        {
            // Windows checks the destination's attribute-write permission on replacement.
            // The flag leaves attributes on the old file and any aliases intact.
            // SAFETY: the same aligned rename header provides the extended flags field.
            unsafe {
                (*info).Anonymous.Flags =
                    FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_IGNORE_READONLY_ATTRIBUTE;
            }
            return rename(FileRenameInformationEx);
        }
        result
    }

    pub(super) fn cleanup(&self, current: Option<&str>, limit: u64) -> io::Result<()> {
        let mut candidates = Vec::new();
        let mut total = 0u128;
        self.for_each_name(|name| {
            let path = Path::new(&name);
            let name_text = name.to_string_lossy();
            let temporary = name_text.starts_with('.') && name_text.ends_with(".tmp");
            let artifact = current.is_some() && is_cache_artifact(path);
            if !(temporary || artifact) {
                return;
            }
            let Ok((file, metadata)) = self.inspect_for_cleanup(&name) else {
                return;
            };
            let modified = metadata.modified().ok();
            if temporary {
                if modified
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age > STALE_TEMPORARY)
                {
                    let _ = delete(&file, metadata.permissions().readonly());
                }
            } else if current.is_some_and(|engine| current_entry(path, engine)) {
                total += u128::from(metadata.len());
                if let (Some(modified), Ok(identity)) = (modified, file_identity(&file)) {
                    candidates.push((modified, name, metadata.len(), identity));
                }
            } else {
                let _ = delete(&file, metadata.permissions().readonly());
            }
        })?;
        if total > u128::from(limit) {
            candidates.sort_unstable();
            for (_, name, size, identity) in candidates {
                if total <= u128::from(limit) {
                    break;
                }
                if let Ok((file, metadata)) = self.inspect_for_cleanup(&name)
                    && file_identity(&file).is_ok_and(|current| current == identity)
                    && delete(&file, metadata.permissions().readonly()).is_ok()
                {
                    total -= u128::from(size);
                }
            }
        }
        Ok(())
    }

    fn inspect_for_cleanup(&self, name: &OsStr) -> io::Result<(File, fs::Metadata)> {
        let open = |access| {
            self.open(
                name,
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                None,
            )
        };
        let access = FILE_READ_ATTRIBUTES | DELETE;
        // Ignoring the read-only attribute requires WRITE_ATTRIBUTES on this same handle.
        let file = open(access | FILE_WRITE_ATTRIBUTES).or_else(|why| {
            if why.kind() == io::ErrorKind::PermissionDenied {
                open(access)
            } else {
                Err(why)
            }
        })?;
        let metadata = file.metadata()?;
        if metadata.file_attributes() & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != 0
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok((file, metadata))
    }

    fn for_each_name(&self, mut visit: impl FnMut(OsString)) -> io::Result<()> {
        const NAME: usize = offset_of!(FILE_NAMES_INFORMATION, FileName);
        // One record per call. The buffer holds a header and an NTFS/ReFS name (255 UTF-16 units).
        let mut buffer = [0u32; 256];
        let mut restart = true;
        loop {
            let mut io_status = IO_STATUS_BLOCK::default();
            // SAFETY: the directory is synchronous, both outputs are live, and no async callback is supplied.
            let status = unsafe {
                NtQueryDirectoryFile(
                    self.file.as_raw_handle(),
                    null_mut(),
                    None,
                    null(),
                    &mut io_status,
                    buffer.as_mut_ptr().cast(),
                    size_of_val(&buffer) as u32,
                    FileNamesInformation,
                    true,
                    null(),
                    restart,
                )
            };
            if status == STATUS_NO_MORE_FILES || (restart && status == STATUS_NO_SUCH_FILE) {
                return Ok(());
            }
            nt_result(status)?;
            restart = false;
            let used = io_status.Information;
            if !(NAME..=size_of_val(&buffer)).contains(&used) {
                return Err(io::ErrorKind::InvalidData.into());
            }
            // The fixed header contains only u32 fields. The name can extend past the declared struct.
            let length = buffer
                [offset_of!(FILE_NAMES_INFORMATION, FileNameLength) / size_of::<u32>()]
                as usize;
            if length > used - NAME || !length.is_multiple_of(size_of::<u16>()) {
                return Err(io::ErrorKind::InvalidData.into());
            }
            // SAFETY: the name starts at an aligned offset and lies inside the reported record.
            let name = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().cast::<u8>().add(NAME).cast::<u16>(),
                    length / size_of::<u16>(),
                )
            };
            visit(OsString::from_wide(name));
        }
    }
}

fn delete(file: &File, read_only: bool) -> io::Result<()> {
    if read_only {
        let info = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        };
        // SAFETY: the handle is live and the buffer matches FileDispositionInfoEx.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfoEx,
                (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
                size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        return Ok(());
    }
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the file owns a live handle and info matches FileDispositionInfo.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(test, windows))]
#[path = "module_cache_windows_tests.rs"]
mod tests;
