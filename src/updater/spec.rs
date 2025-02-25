#[cfg(test)]
mod test {
    use crate::{
        message::ContractType,
        spec::{
            MintBurnAssetCollateralizedSpec, MintBurnAssetSpec, SpecContract, SpecContractType,
        },
        Flaw,
    };
    use super::{
        mint_burn_asset::{
            BurnMechanisms, Collateralized, MBAMintMechanisms, MintBurnAssetContract,
            MintStructure, ProportionalType, RatioModel, ReturnCollateral, SwapMechanisms,
        },
        InputAsset, Updater,
    };

    #[test]
    pub fn validate_contract_spec_mba_valid_constant_product() {
        let mba_contract = ContractType::Mba(MintBurnAssetContract {
            ticker: None,
            supply_cap: None,
            divisibility: 18,
            live_time: 0,
            end_time: None,
            mint_mechanism: MBAMintMechanisms {
                preallocated: None,
                free_mint: None,
                purchase: None,
                collateralized: Some(Collateralized {
                    input_assets: vec![InputAsset::Rune, InputAsset::Ordinal],
                    _mutable_assets: false,
                    mint_structure: MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: Some(100),
                    }),
                }),
            },
            burn_mechanism: BurnMechanisms {
                return_collateral: Some(ReturnCollateral {
                    fee: None,
                    oracle_setting: None,
                }),
            },
            swap_mechanism: SwapMechanisms { fee: None },
            commitment: None,
        });

        let spec_mba_contract = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                collateralized: Some(MintBurnAssetCollateralizedSpec {
                    _mutable_assets: true,
                    input_assets: Some(vec![InputAsset::Rune, InputAsset::Ordinal]),
                    mint_structure: Some(MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: Some(100),
                    })),
                }),
            }),
            block_tx: None,
            pointer: None,
        };

        let flaw = Updater::validate_contract_by_spec(spec_mba_contract, &mba_contract);
        assert_eq!(flaw, None);
    }

    #[test]
    pub fn validate_contract_spec_mba_input_assets_invalid() {
        let mba_contract = ContractType::Mba(MintBurnAssetContract {
            ticker: None,
            supply_cap: None,
            divisibility: 18,
            live_time: 0,
            end_time: None,
            mint_mechanism: MBAMintMechanisms {
                preallocated: None,
                free_mint: None,
                purchase: None,
                collateralized: Some(Collateralized {
                    input_assets: vec![InputAsset::Rune, InputAsset::RawBtc],
                    _mutable_assets: false,
                    mint_structure: MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: None,
                    }),
                }),
            },
            burn_mechanism: BurnMechanisms {
                return_collateral: Some(ReturnCollateral {
                    fee: None,
                    oracle_setting: None,
                }),
            },
            swap_mechanism: SwapMechanisms { fee: None },
            commitment: None,
        });

        let spec_mba_contract = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                collateralized: Some(MintBurnAssetCollateralizedSpec {
                    _mutable_assets: true,
                    input_assets: Some(vec![InputAsset::Rune]),
                    mint_structure: Some(MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: None,
                    })),
                }),
            }),
            block_tx: None,
            pointer: None,
        };

        let flaw = Updater::validate_contract_by_spec(spec_mba_contract, &mba_contract);
        assert_eq!(flaw, Some(Flaw::SpecCriteriaInvalid));
    }

    #[test]
    pub fn validate_contract_spec_mba_mint_structure_invalid() {
        let mba_contract = ContractType::Mba(MintBurnAssetContract {
            ticker: None,
            supply_cap: None,
            divisibility: 18,
            live_time: 0,
            end_time: None,
            mint_mechanism: MBAMintMechanisms {
                preallocated: None,
                free_mint: None,
                purchase: None,
                collateralized: Some(Collateralized {
                    input_assets: vec![InputAsset::Rune, InputAsset::RawBtc],
                    _mutable_assets: false,
                    mint_structure: MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: None,
                    }),
                }),
            },
            burn_mechanism: BurnMechanisms {
                return_collateral: Some(ReturnCollateral {
                    fee: None,
                    oracle_setting: None,
                }),
            },
            swap_mechanism: SwapMechanisms { fee: None },
            commitment: None,
        });

        let spec_mba_contract = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                collateralized: Some(MintBurnAssetCollateralizedSpec {
                    _mutable_assets: true,
                    input_assets: Some(vec![InputAsset::Rune]),
                    mint_structure: Some(MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: Some(100),
                    })),
                }),
            }),
            block_tx: None,
            pointer: None,
        };

        let flaw = Updater::validate_contract_by_spec(spec_mba_contract, &mba_contract);
        assert_eq!(flaw, Some(Flaw::SpecCriteriaInvalid));
    }

    #[test]
    pub fn validate_contract_spec_mba_valid_constant_sum() {
        let mba_contract = ContractType::Mba(MintBurnAssetContract {
            ticker: None,
            supply_cap: None,
            divisibility: 18,
            live_time: 0,
            end_time: None,
            mint_mechanism: MBAMintMechanisms {
                preallocated: None,
                free_mint: None,
                purchase: None,
                collateralized: Some(Collateralized {
                    input_assets: vec![InputAsset::Rune, InputAsset::Ordinal],
                    _mutable_assets: false,
                    mint_structure: MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantSum,
                        inital_mint_pointer_to_key: Some(100),
                    }),
                }),
            },
            burn_mechanism: BurnMechanisms {
                return_collateral: Some(ReturnCollateral {
                    fee: None,
                    oracle_setting: None,
                }),
            },
            swap_mechanism: SwapMechanisms { fee: None },
            commitment: None,
        });

        let spec_mba_contract = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                collateralized: Some(MintBurnAssetCollateralizedSpec {
                    _mutable_assets: true,
                    input_assets: Some(vec![InputAsset::Rune, InputAsset::Ordinal]),
                    mint_structure: Some(MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantSum,
                        inital_mint_pointer_to_key: Some(100),
                    })),
                }),
            }),
            block_tx: None,
            pointer: None,
        };

        let flaw = Updater::validate_contract_by_spec(spec_mba_contract, &mba_contract);
        assert_eq!(flaw, None);
    }

    #[test]
    pub fn validate_contract_spec_mba_mint_structure_invalid_constant_sum() {
        // Here, the MBA contract uses ConstantSum while the spec expects ConstantProduct.
        let mba_contract = ContractType::Mba(MintBurnAssetContract {
            ticker: None,
            supply_cap: None,
            divisibility: 18,
            live_time: 0,
            end_time: None,
            mint_mechanism: MBAMintMechanisms {
                preallocated: None,
                free_mint: None,
                purchase: None,
                collateralized: Some(Collateralized {
                    input_assets: vec![InputAsset::Rune, InputAsset::Ordinal],
                    _mutable_assets: false,
                    mint_structure: MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantSum,
                        inital_mint_pointer_to_key: Some(100),
                    }),
                }),
            },
            burn_mechanism: BurnMechanisms {
                return_collateral: Some(ReturnCollateral {
                    fee: None,
                    oracle_setting: None,
                }),
            },
            swap_mechanism: SwapMechanisms { fee: None },
            commitment: None,
        });

        let spec_mba_contract = SpecContract {
            spec: SpecContractType::MintBurnAsset(MintBurnAssetSpec {
                collateralized: Some(MintBurnAssetCollateralizedSpec {
                    _mutable_assets: true,
                    input_assets: Some(vec![InputAsset::Rune, InputAsset::Ordinal]),
                    mint_structure: Some(MintStructure::Proportional(ProportionalType {
                        ratio_model: RatioModel::ConstantProduct,
                        inital_mint_pointer_to_key: Some(100),
                    })),
                }),
            }),
            block_tx: None,
            pointer: None,
        };

        let flaw = Updater::validate_contract_by_spec(spec_mba_contract, &mba_contract);
        assert_eq!(flaw, Some(Flaw::SpecCriteriaInvalid));
    }
}
