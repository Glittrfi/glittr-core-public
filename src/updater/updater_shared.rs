use message::OracleMessageSigned;
use transaction_shared::OracleSetting;

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

impl Updater {
    pub fn validate_pointer(&self, pointer: u32, tx: &Transaction) -> Option<Flaw> {
        if pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        if self.is_op_return_index(&tx.output[pointer as usize]) {
            return Some(Flaw::InvalidPointer);
        }
        None
    }

    pub fn validate_oracle_message(
        &self,
        oracle_message: &OracleMessageSigned,
        setting: &OracleSetting,
        block_tx: &BlockTx,
    ) -> Option<Flaw> {
        // Check asset ID matches
        if setting.asset_id.is_some() {
            if setting.asset_id != oracle_message.message.asset_id {
                return Some(Flaw::OracleMintFailed);
            }
        }

        // Check block height slippage
        if block_tx.block - oracle_message.message.block_height
            > setting.block_height_slippage as u64
        {
            return Some(Flaw::OracleMintBlockSlippageExceeded);
        }

        // Validate signature
        let pubkey = XOnlyPublicKey::from_slice(&setting.pubkey).unwrap();

        let msg = Message::from_digest_slice(
            sha256::Hash::hash(
                serde_json::to_string(&oracle_message.message)
                    .unwrap()
                    .as_bytes(),
            )
            .as_byte_array(),
        )
        .unwrap();

        let signature = Signature::from_slice(&oracle_message.signature);

        if signature.is_err() {
            return Some(Flaw::OracleMintSignatureFailed);
        }

        if pubkey
            .verify(&Secp256k1::new(), &msg, &signature.unwrap())
            .is_err()
        {
            return Some(Flaw::OracleMintSignatureFailed);
        }

        None
    }
}
