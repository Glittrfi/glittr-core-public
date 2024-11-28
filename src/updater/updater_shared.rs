use super::*;

pub fn relative_block_height_to_block_height(
    block_height_relative_absolute: RelativeOrAbsoluteBlockHeight,
    current_block_height: BlockHeight,
) -> BlockHeight {
    if block_height_relative_absolute < 0 {
        current_block_height.saturating_add(-block_height_relative_absolute as u64)
    } else {
        block_height_relative_absolute as u64
    }
}
