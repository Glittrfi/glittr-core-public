use std::str::FromStr;

use asset_contract::{AssetContract, AssetContractPurchaseBurnSwap, InputAsset};
use bitcoin::{
    hashes::{sha256, Hash},
    key::Secp256k1,
    opcodes,
    script::Instruction,
    secp256k1::{schnorr::Signature, Message},
    Address, Transaction, XOnlyPublicKey,
};
use database::{DatabaseError, MESSAGE_PREFIX, TRANSACTION_TO_BLOCK_TX_PREFIX};
use flaw::Flaw;
use message::{CallType, ContractType, MintOption, OpReturnMessage, TxType};

use super::*;

#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct MessageDataOutcome {
    pub message: Option<OpReturnMessage>,
    pub flaw: Option<Flaw>,
}

pub struct MintResult {
    pub out_value: u128,
    pub txout: u32,
}

pub struct Updater {
    pub database: Arc<Mutex<Database>>,
}

impl Updater {
    pub async fn new(database: Arc<Mutex<Database>>) -> Self {
        Updater { database }
    }

    // run modules here
    pub async fn index(
        &mut self,
        block_height: u64,
        tx_index: u32,
        tx: &Transaction,
        message_result: Result<OpReturnMessage, Flaw>,
    ) -> Result<(), Box<dyn Error>> {
        let mut outcome = MessageDataOutcome {
            message: None,
            flaw: None,
        };

        let block_tx = &BlockTx {
            block: block_height,
            tx: tx_index,
        };

        if let Ok(message) = message_result {
            outcome.message = Some(message.clone());
            if let Some(flaw) = message.validate() {
                outcome.flaw = Some(flaw);
            } else {
                outcome.flaw = match message.tx_type {
                    TxType::Transfer {
                        asset: _,
                        n_outputs: _,
                        amounts: _,
                    } => {
                        log::info!("Process transfer");
                        None
                    }
                    TxType::ContractCreation { contract_type } => {
                        log::info!("Process contract creation");
                        if let ContractType::Asset(asset_contract) = contract_type {
                            if let AssetContract::PurchaseBurnSwap(
                                AssetContractPurchaseBurnSwap { input_asset, .. },
                            ) = asset_contract
                            {
                                if let InputAsset::GlittrAsset(block_tx_tuple) = input_asset {
                                    if let Some(tx_type) =
                                        self.get_message_txtype(block_tx_tuple).await.ok()
                                    {
                                        match tx_type {
                                            TxType::ContractCreation { .. } => None,
                                            _ => Some(Flaw::ReferencingFlawedBlockTx),
                                        };
                                    }
                                }
                            }
                        }
                        None
                    }
                    TxType::ContractCall {
                        contract,
                        call_type,
                    } => match call_type {
                        CallType::Mint(mint_option) => self.mint(tx, contract, mint_option).await,
                        CallType::Burn => {
                            log::info!("Process call type burn");
                            None
                        }
                        CallType::Swap => {
                            log::info!("Process call type swap");
                            None
                        }
                    },
                }
            }
        } else {
            outcome.flaw = Some(message_result.unwrap_err());
        }

        self.database
            .lock()
            .await
            .put(MESSAGE_PREFIX, block_tx.to_string().as_str(), outcome)?;

        self.database.lock().await.put(
            TRANSACTION_TO_BLOCK_TX_PREFIX,
            tx.compute_txid().to_string().as_str(),
            block_tx.to_tuple(),
        )?;

        Ok(())
    }

    async fn get_message_txtype(&self, block_tx: BlockTxTuple) -> Result<TxType, Flaw> {
        let outcome: MessageDataOutcome = self
            .database
            .lock()
            .await
            .get(
                MESSAGE_PREFIX,
                BlockTx {
                    block: block_tx.0,
                    tx: block_tx.1,
                }
                .to_string()
                .as_str(),
            )
            .unwrap();

        if outcome.flaw.is_some() {
            return Err(Flaw::ReferencingFlawedBlockTx);
        } else {
            return Ok(outcome.message.unwrap().tx_type);
        }
    }

    async fn get_message(&self, contract_id: &BlockTxTuple) -> Result<OpReturnMessage, Flaw> {
        let contract_key = BlockTx::from_tuple(*contract_id).to_string();
        let outcome: Result<MessageDataOutcome, DatabaseError> = self
            .database
            .lock()
            .await
            .get(MESSAGE_PREFIX, &contract_key);

        match outcome {
            Ok(outcome) => {
                if let Some(flaw) = outcome.flaw {
                    Err(flaw)
                } else {
                    outcome.message.ok_or(Flaw::MessageInvalid)
                }
            }
            Err(DatabaseError::NotFound) => Err(Flaw::ContractNotFound),
            Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
        }
    }

    pub async fn get_mint_purchase_burn_swap(
        pbs: AssetContractPurchaseBurnSwap,
        tx: &Transaction,
        mint_option: MintOption,
    ) -> Result<MintResult, Flaw> {
        // TODO: given the utxo, how you would know if the utxo contains glittr asset. will be implemented on transfer.
        let mut total_received_value: u128 = 0;
        let mut out_value: u128 = 0;
        let mut txout: Option<u32> = None;

        // VALIDATE OUTPUT
        match pbs.transfer_scheme {
            // Ensure that the asset is set to burn
            asset_contract::TransferScheme::Burn => {
                for (pos, output) in tx.output.iter().enumerate() {
                    let mut instructions = output.script_pubkey.instructions();

                    if instructions.next() == Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
                        match pbs.input_asset {
                            InputAsset::RawBTC => {
                                total_received_value = output.value.to_sat() as u128;
                            }
                            InputAsset::GlittrAsset(_) => todo!(),
                            InputAsset::Metaprotocol => {
                                // TODO: handle transfer for each and every metaprotocol (ordinal, runes)
                                // need to copy the transfer mechanics for each, make sure the sat is burned
                            }
                        }

                        if mint_option.pointer != pos as u32 {
                            // NOTE: all asset always went to the first non-op-return txout if the pointer is invalid (for burn)
                            txout = Some(mint_option.pointer as u32);
                            break;
                        }
                    } else {
                        if txout.is_none() {
                            txout = Some(pos as u32);
                        }
                    }
                }
            }
            asset_contract::TransferScheme::Purchase(bitcoin_address) => {
                for output in tx.output.iter() {
                    let address = Address::from_str(bitcoin_address.as_str())
                        .unwrap()
                        .assume_checked();
                    // TODO: bitcoin network from CONFIG
                    let address_from_script = Address::from_script(
                        output.script_pubkey.as_script(),
                        bitcoin::Network::Regtest,
                    );

                    if let Some(address_from_script) = address_from_script.ok() {
                        if address == address_from_script {
                            match pbs.input_asset {
                                InputAsset::RawBTC => {
                                    total_received_value = output.value.to_sat() as u128;
                                }
                                InputAsset::GlittrAsset(_) => todo!(),
                                InputAsset::Metaprotocol => {
                                    // TODO: handle transfer for each and every metaprotocol (ordinal, runes)
                                    // need to copy the transfer mechanics for each, make sure the sat is burned
                                }
                            }
                        }
                    }
                }
                txout = Some(mint_option.pointer);
            }
        }

        if txout.unwrap() > tx.output.len() as u32 {
            return Err(Flaw::InvalidMintPointer);
        }

        // VALIDATE INPUT
        match pbs.input_asset {
            InputAsset::GlittrAsset(_) => todo!(),
            _ => (), // validated below
        };

        if let asset_contract::TransferRatioType::Fixed { ratio } = pbs.transfer_ratio_type {
            out_value = (total_received_value as u128 * ratio.0) / ratio.1;
        } else if let asset_contract::TransferRatioType::Oracle { pubkey, setting } =
            pbs.transfer_ratio_type.clone()
        {
            let verified = if let Some(oracle_message_signed) = mint_option.oracle_message {
                if setting.asset_id == oracle_message_signed.message.asset_id {
                    let pubkey: XOnlyPublicKey =
                        XOnlyPublicKey::from_slice(pubkey.as_slice()).unwrap();

                    if let Some(signature) =
                        Signature::from_slice(&oracle_message_signed.signature).ok()
                    {
                        let secp = Secp256k1::new();

                        let msg = Message::from_digest_slice(
                            sha256::Hash::hash(
                                serde_json::to_string(&oracle_message_signed.message)
                                    .unwrap()
                                    .as_str()
                                    .as_bytes(),
                            )
                            .as_byte_array(),
                        )
                        .unwrap();

                        out_value = oracle_message_signed.message.out_value as u128;

                        let mut input_found = false;
                        for txin in tx.input.iter() {
                            if txin.previous_output == oracle_message_signed.message.input_outpoint
                            {
                                input_found = true;
                            }
                        }

                        let below_min_in_value =
                            total_received_value < oracle_message_signed.message.min_in_value;

                        !below_min_in_value
                            && input_found
                            && pubkey.verify(&secp, &msg, &signature).is_ok()
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if !verified {
                return Err(Flaw::OracleMintFailed);
            }
        }

        // TODO: save the mint result on asset tracker (wait for jon)
        Ok(MintResult {
            out_value,
            txout: txout.unwrap(),
        })
    }

    async fn mint(
        &mut self,
        tx: &Transaction,
        contract_id: BlockTxTuple,
        mint_option: MintOption,
    ) -> Option<Flaw> {
        let message = self.get_message(&contract_id).await;
        match message {
            Ok(op_return_message) => match op_return_message.tx_type {
                TxType::ContractCreation { contract_type } => match contract_type {
                    message::ContractType::Asset(asset) => match asset {
                        AssetContract::PurchaseBurnSwap(pbs) => {
                            Updater::get_mint_purchase_burn_swap(pbs, tx, mint_option)
                                .await
                                .err()
                        }
                        _ => Some(Flaw::ContractNotMatch),
                    },
                },
                _ => Some(Flaw::ContractNotMatch),
            },
            Err(flaw) => Some(flaw),
        }
    }
}
