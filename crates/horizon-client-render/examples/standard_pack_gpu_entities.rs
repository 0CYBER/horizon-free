#[cfg(not(test))]
#[path = "../fixtures/standard_pack_gpu_entities.rs"]
mod fixture;

#[cfg(not(test))]
fn main() {
    fixture::main();
}
