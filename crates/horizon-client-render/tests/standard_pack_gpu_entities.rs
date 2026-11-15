#[path = "../fixtures/standard_pack_gpu_entities.rs"]
mod fixture;

#[test]
#[ignore = "daily"]
fn standard_pack_gpu_entities() {
    fixture::run_daily();
}
