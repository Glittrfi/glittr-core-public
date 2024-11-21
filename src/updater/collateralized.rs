use super::*;
use crate::updater::database::COLLATERAL_ACCOUNT_PREFIX;
use bitcoin::Transaction;
use message::OpenAccountOption;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CollateralAccount {
    pub collateral_amounts: Vec<(BlockTxTuple, u128)>,
    // TODO: using the share amount, and asserting if share amount relative to the global share
    pub share_amount: u128
}

impl Updater {
    // call together with transfer
    pub async fn process_open_account(
        &mut self,
        tx: &Transaction,
        _block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        open_account_option: &OpenAccountOption,
    ) -> Option<Flaw> {
        // Get the MBA contract
        let message = self.get_message(contract_id).await;
        let mut collateral_amounts = Vec::new();

        match message {
            Ok(op_return_message) => {
                let contract_creation = op_return_message.contract_creation?;
                match contract_creation.contract_type {
                    ContractType::Mba(mba) => {
                        if let Some(collateralized) = mba.mint_mechanism.collateralized {
                            for input_asset in collateralized.input_assets {
                                if let InputAsset::GlittrAsset(asset_id) = input_asset {
                                    let burned_amount = self
                                        .unallocated_asset_list
                                        .list
                                        .remove(&BlockTx::from_tuple(asset_id).to_str())
                                        .unwrap_or(0);

                                    if burned_amount > 0 {
                                        collateral_amounts.push(((asset_id), burned_amount));
                                    }
                                }
                            }
                        }
                    }
                    _ => return Some(Flaw::InvalidContractType),
                };
            }
            Err(_) => return Some(Flaw::ContractNotMatch),
        }

        let collateral_account = CollateralAccount { collateral_amounts, share_amount: open_account_option.share_amount.0 };

        if let Some(flaw) = self.validate_pointer(open_account_option.pointer, tx) {
            return Some(flaw);
        }

        let txid = tx.compute_txid().to_string();
        let outpoint = &Outpoint {
            txid,
            vout: open_account_option.pointer,
        };

        if !self.is_read_only {
            self.database.lock().await.put(
                COLLATERAL_ACCOUNT_PREFIX,
                outpoint.to_string().as_str(),
                collateral_account,
            );
        }

        None
    }
}
