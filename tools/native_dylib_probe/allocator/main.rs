//! Allocates in one image and frees in another; prints one JSON row per case.
type Make = fn() -> Vec<u8>;
type Free = fn(Vec<u8>);
fn exe_make() -> Vec<u8> {
    std::hint::black_box(vec![1; 4096])
}
fn exe_free(value: Vec<u8>) {
    drop(std::hint::black_box(value))
}
fn main() {
    let images: [(&str, Make, Free); 3] = [
        ("game", game::make, game::free),
        ("mod", modx::make, modx::free),
        ("executable", exe_make, exe_free),
    ];
    let mut rows = Vec::new();
    let mut mismatched = false;
    for (allocated_in, make, _) in images {
        for (freed_in, _, free) in images {
            if allocated_in == freed_in {
                continue;
            }
            let before = game::foreign_frees();
            let value = make();
            let pointer = value.as_ptr();
            let by_game = game::owned(pointer);
            free(value);
            let still_live = game::owned(pointer);
            let foreign = game::foreign_frees() - before;
            mismatched |= !by_game || still_live || foreign != 0;
            rows.push(format!(
                r#"{{"allocated_in":"{allocated_in}","freed_in":"{freed_in}","allocated_by_game_allocator":{by_game},"left_live_in_game_allocator":{still_live},"game_allocator_foreign_frees":{foreign}}}"#
            ));
        }
    }
    let boxed = modx::make_box();
    let by_game = game::owned(boxed.as_ptr());
    modx::game_box_free(boxed);
    mismatched |= !by_game;
    rows.push(format!(
        r#"{{"allocated_in":"mod Box","allocated_by_game_allocator":{by_game}}}"#
    ));
    println!("[{}]", rows.join(","));
    println!(
        "{}",
        if mismatched {
            "ALLOCATOR MISMATCH"
        } else {
            "ONE ALLOCATOR"
        }
    );
}
