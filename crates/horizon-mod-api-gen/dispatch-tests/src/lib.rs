#[cfg(test)]
include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_crate_root.rs"));

#[cfg(test)]
mod tests {
    #[allow(dead_code)]
    mod stubs {
        include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_stubs.rs"));
    }
    use generated as mod_api;
    include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_root.rs"));
}
