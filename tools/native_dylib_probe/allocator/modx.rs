//! Stands in for a mod library: allocates and frees with whatever allocator its imports bind to.
pub fn make() -> Vec<u8> {
    std::hint::black_box(vec![9; 4096])
}
pub fn free(value: Vec<u8>) {
    drop(std::hint::black_box(value))
}
pub fn make_box() -> Box<[u8; 64]> {
    std::hint::black_box(Box::new([3; 64]))
}
pub fn game_box_free(value: Box<[u8; 64]>) {
    drop(std::hint::black_box(value))
}
