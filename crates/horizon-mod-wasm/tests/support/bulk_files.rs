use mod_api::{ConfigEntry, ConfigEntryKind, ConfigFileError, ConfigFiles, ConfigPath, ConfigStat};
use std::collections::BTreeMap;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub struct Files {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    pub no_world: AtomicBool,
}
impl Files {
    const fn new() -> Self {
        Self {
            files: Mutex::new(BTreeMap::new()),
            no_world: AtomicBool::new(false),
        }
    }
    fn available(&self) -> Result<(), ConfigFileError> {
        if self.no_world.load(Ordering::Relaxed) {
            Err(ConfigFileError::NoWorld)
        } else {
            Ok(())
        }
    }
}
fn stat(len: usize) -> ConfigStat {
    mod_api::__private::built::ConfigStat {
        kind: ConfigEntryKind::File,
        len: len as u64,
        modified_millis: 17,
    }
    .into()
}
impl ConfigFiles for Files {
    fn read(&self, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError> {
        self.available()?;
        self.files
            .lock()
            .unwrap()
            .get(file.as_str())
            .cloned()
            .ok_or(ConfigFileError::NotFound)
    }
    fn write(&self, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError> {
        self.available()?;
        self.files
            .lock()
            .unwrap()
            .insert(file.as_str().into(), bytes.to_vec());
        Ok(())
    }
    fn list(
        &self,
        folder: &ConfigPath,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ConfigEntry>, ConfigFileError> {
        self.available()?;
        if limit == 0 || limit as usize > mod_api::CONFIG_LIST_MAX {
            return Err(ConfigFileError::InvalidPage);
        }
        let prefix = if folder.as_str().is_empty() {
            String::new()
        } else {
            format!("{}/", folder.as_str())
        };
        Ok(self
            .files
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(path, bytes)| {
                let name = path.strip_prefix(&prefix)?;
                (!name.contains('/') && after.is_none_or(|after| name > after)).then(|| {
                    mod_api::__private::built::ConfigEntry {
                        name: name.into(),
                        stat: stat(bytes.len()),
                    }
                    .into()
                })
            })
            .take(limit as usize)
            .collect())
    }
    fn stat(&self, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError> {
        self.available()?;
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(path.as_str())
            .map(|bytes| stat(bytes.len())))
    }
}

static CONFIG: Files = Files::new();
static DATA: Files = Files::new();
pub struct World {
    pub world: Files,
}
impl Default for World {
    fn default() -> Self {
        Self {
            world: Files::new(),
        }
    }
}
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World, except(config));
impl mod_api::prelude::ConfigHost for World {
    fn config_files(&self) -> &'static dyn ConfigFiles {
        &CONFIG
    }
    fn data_files(&self) -> &'static dyn ConfigFiles {
        &DATA
    }
}
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(config));
impl mod_api::prelude::ConfigServerHost for World {
    fn world_data_files(&self) -> &dyn ConfigFiles {
        &self.world
    }
}
