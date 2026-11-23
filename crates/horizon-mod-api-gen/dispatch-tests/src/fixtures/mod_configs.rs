mod stubs {
    #[derive(Clone)]
    pub(super) struct Config;
    pub(super) struct ConfigFile;
}
mod crate_root {
    // Dispatch fixtures isolate ordering; API tests cover the ownership check.
    mod content {
        pub(crate) mod config_files {
            pub(crate) fn own_configs<T>(_: &str, _: &[T], _: &mut Vec<T>) {}
        }
    }
}
