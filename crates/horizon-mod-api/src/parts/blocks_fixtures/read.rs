#[cfg(test)]
/// The comparator reading of state `state`, from 1.
pub(crate) fn read(state: u32) -> i32 {
    let mut output = 1;
    dispatch_analog_output(
        &Level,
        LevelDimension::Overworld,
        BlockPos::new(0, 64, 0),
        value(state),
        Direction::North,
        &mut output,
    );
    output
}
