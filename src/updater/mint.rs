use message::MintOption;

use super::*;

impl Updater {
    async fn get_mint_data(&self, contract_id: &BlockTxTuple) -> Result<MintData, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let result: Result<MintData, DatabaseError> = self
            .database
            .lock()
            .await
            .get(MINT_DATA_PREFIX, &contract_key);

        match result {
            Ok(data) => Ok(data),
            Err(DatabaseError::NotFound) => Ok(MintData::default()),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    async fn set_mint_output(
        &self,
        contract_id: &BlockTxTuple,
        tx_id: &str,
        n_output: u32,
        amount: u32,
    ) {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let key = format!("{}:{}:{}", contract_key, tx_id, n_output.to_string());
        self.database
            .lock()
            .await
            .put(MINT_OUTPUT_PREFIX, &key, amount);
    }

    async fn update_mint_data(&self, contract_id: &BlockTxTuple, mint_data: &MintData) {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        self.database
            .lock()
            .await
            .put(MINT_DATA_PREFIX, &contract_key, mint_data);
    }

    async fn mint_free_mint(
        &self,
        asset: AssetContractFreeMint,
        tx: &Transaction,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        let mut mint_data = match self.get_mint_data(contract_id).await {
            Ok(data) => data,
            Err(flaw) => return Some(flaw),
        };

        // check the supply
        if let Some(supply_cap) = asset.supply_cap {
            let next_supply = mint_data
                .minted
                .saturating_mul(asset.amount_per_mint)
                .saturating_add(asset.amount_per_mint);

            if next_supply > supply_cap {
                return Some(Flaw::SupplyCapExceeded);
            }
        }
        mint_data.minted = mint_data.minted.saturating_add(1);

        // check pointer overflow
        if mint_option.pointer >= tx.output.len() as u32 {
            return Some(Flaw::PointerOverflow);
        }
        // TODO: check livetime

        // set the outpoint
        self.set_mint_output(
            contract_id,
            tx.compute_txid().to_string().as_str(),
            mint_option.pointer,
            asset.amount_per_mint,
        )
        .await;

        // update the mint data
        self.update_mint_data(contract_id, &mint_data).await;

        None
    }

    pub async fn mint(
        &mut self,
        tx: &Transaction,
        contract_id: &BlockTxTuple,
        mint_option: &MintOption,
    ) -> Option<Flaw> {
        let message = self.get_message(&contract_id).await;
        match message {
            Ok(op_return_message) => match op_return_message.tx_type {
                TxType::ContractCreation { contract_type } => match contract_type {
                    message::ContractType::Asset(asset) => match asset {
                        AssetContract::FreeMint(free_mint) => {
                            self.mint_free_mint(free_mint, tx, contract_id, mint_option)
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
