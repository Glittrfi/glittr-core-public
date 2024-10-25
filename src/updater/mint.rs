use message::MintOption;

use super::*;

impl Updater {
    async fn get_free_mint_data(
        &self,
        contract_id: &BlockTxTuple,
    ) -> Result<AssetContractDataFreeMint, Flaw> {
        let data = match self.get_asset_contract_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => match flaw {
                Flaw::AssetContractDataNotFound => return Ok(AssetContractDataFreeMint::default()),
                _ => return Err(flaw),
            },
        };

        match data {
            AssetContractData::FreeMint(free_mint) => Ok(free_mint),
            // TODO: add error when enum type doesn't match
        }
    }

    async fn update_asset_list_for_mint(
        &self,
        contract_id: &BlockTxTuple,
        outpoint: &Outpoint,
        amount: u32,
    ) -> Option<Flaw> {
        let mut asset_list = match self.get_asset_list(outpoint).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };
        let block_tx = BlockTx::from_tuple(*contract_id);
        let previous_amount = asset_list.list.get(&block_tx.to_str()).unwrap_or(&0);

        asset_list
            .list
            .insert(block_tx.to_str(), previous_amount.saturating_add(amount));
        self.set_asset_list(outpoint, &asset_list).await;

        None
    }

    async fn update_asset_contract_data_for_free_mint(
        &self,
        contract_id: &BlockTxTuple,
        free_mint_data: &AssetContractDataFreeMint,
    ) {
        self.set_asset_contract_data(
            &contract_id,
            &AssetContractData::FreeMint(free_mint_data.clone()),
        )
        .await
    }

    async fn mint_free_mint(
        &self,
        asset: AssetContractFreeMint,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        // check livetime
        if asset.live_time > block_tx.block  {
            return Some(Flaw::LiveTimeNotReached);
        }

        let mut free_mint_data = match self.get_free_mint_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        // check the supply
        if let Some(supply_cap) = asset.supply_cap {
            let next_supply = free_mint_data
                .minted
                .saturating_mul(asset.amount_per_mint)
                .saturating_add(asset.amount_per_mint);

            if next_supply > supply_cap {
                return Some(Flaw::SupplyCapExceeded);
            }
        }
        free_mint_data.minted = free_mint_data.minted.saturating_add(1);

        // check pointer overflow
        if mint_option.pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        let outpoint = Outpoint {
            txid: tx.compute_txid().to_string(),
            vout: mint_option.pointer,
        };

        // set the outpoint
        let flaw = self
            .update_asset_list_for_mint(contract_id, &outpoint, asset.amount_per_mint)
            .await;
        if flaw.is_some() {
            return flaw;
        }

        // update the mint data
        self.update_asset_contract_data_for_free_mint(contract_id, &free_mint_data)
            .await;

        None
    }

    pub async fn mint(
        &mut self,
        tx: &Transaction,
        block_tx: &BlockTx,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        let message = self.get_message(&contract_id).await;
        match message {
            Ok(op_return_message) => match op_return_message.tx_type {
                TxType::ContractCreation { contract_type } => match contract_type {
                    message::ContractType::Asset(asset) => match asset {
                        AssetContract::FreeMint(free_mint) => {
                            self.mint_free_mint(free_mint, tx, block_tx, contract_id, mint_option)
                                .await
                        }
                        // TODO: add others asset types
                        _ => Some(Flaw::ContractNotMatch),
                    },
                },
                _ => Some(Flaw::ContractNotMatch),
            },
            Err(flaw) => Some(flaw),
        }
    }
}
