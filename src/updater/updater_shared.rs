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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CollateralAccount {
    pub collateral_amounts: Vec<(BlockTxTuple, u128)>,
    pub total_collateral_amount: u128,
    pub ltv: Fraction, // ltv = total_amount_used / total_collateral_amount (in lending amount)
    pub amount_outstanding: u128,
    pub share_amount: u128, // TODO: implement
}