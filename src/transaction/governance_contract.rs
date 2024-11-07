use bitcoin::XOnlyPublicKey;
use flaw::Flaw;
use super::*;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct GovernanceContract {
    pub governance_token: BlockTxTuple,
    pub governance_settings: GovernanceSettings,
    pub proposal_settings: ProposalSettings,
    pub execution_settings: ExecutionSettings,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct GovernanceSettings {
    /// Minimum balance required to create a proposal
    pub proposal_threshold: U128,
    /// Minimum percentage of total supply that must vote for proposal to be valid
    pub quorum_ratio: Ratio,
    /// Minimum percentage of votes that must be "yes" for proposal to pass
    pub approval_ratio: Ratio,
    /// Optional delegation system
    pub allow_delegation: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ProposalSettings {
    /// How long proposals can be voted on (in blocks)
    pub voting_period: BlockHeight,
    /// Optional delay before voting starts
    pub voting_delay: Option<BlockHeight>,
    /// Whether votes can be changed during voting period
    pub allow_vote_changes: bool,
    /// Optional early execution if quorum and approval threshold met
    pub allow_early_execution: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ExecutionSettings {
    /// Delay between proposal passing and execution
    pub execution_delay: BlockHeight,
    /// Time window during which execution must occur
    pub execution_window: BlockHeight,
    /// Optional list of authorized executors (if None, anyone can execute)
    pub executors: Option<Vec<Pubkey>>,
    /// Optional veto powers
    pub veto_powers: Option<VetoPowers>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct VetoPowers {
    pub veto_authorities: Vec<Pubkey>,
    /// How many veto authorities need to agree
    pub required_vetoes: u32,
    /// Time window for veto after proposal passes
    pub veto_window: BlockHeight,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum VoteType {
    Simple {
        options: Vec<String>,
    },
    Ranked {
        options: Vec<String>,
        max_ranks: u8,
    },
    Weighted {
        options: Vec<String>,
        max_weight_per_option: U128,
    },
}

impl GovernanceContract {
    pub fn validate(&self) -> Option<Flaw> {
        // Validate governance token exists
        if self.governance_token.1 == 0 {
            return Some(Flaw::InvalidBlockTxPointer);
        }

        // Validate ratios
        if self.governance_settings.quorum_ratio.1 == 0 
            || self.governance_settings.approval_ratio.1 == 0 {
            return Some(Flaw::DivideByZero);
        }

        // Validate ratio values are reasonable
        if self.governance_settings.quorum_ratio.0 > self.governance_settings.quorum_ratio.1 
            || self.governance_settings.approval_ratio.0 > self.governance_settings.approval_ratio.1 {
            return Some(Flaw::InvalidRatio);
        }

        // Validate timing parameters
        if self.proposal_settings.voting_period == 0 
            || self.execution_settings.execution_window == 0 {
            return Some(Flaw::InvalidBlockHeight);
        }

        // Validate veto settings if present
        if let Some(veto_powers) = &self.execution_settings.veto_powers {
            if veto_powers.veto_authorities.is_empty() 
                || veto_powers.required_vetoes == 0 
                || veto_powers.required_vetoes as usize > veto_powers.veto_authorities.len() {
                return Some(Flaw::InvalidVetoConfiguration);
            }

            // Validate veto authority pubkeys
            for pubkey in &veto_powers.veto_authorities {
                if XOnlyPublicKey::from_slice(pubkey).is_err() {
                    return Some(Flaw::PubkeyInvalid);
                }
            }
        }

        // Validate executors if present
        if let Some(executors) = &self.execution_settings.executors {
            if executors.is_empty() {
                return Some(Flaw::InvalidExecutorConfiguration);
            }

            for pubkey in executors {
                if XOnlyPublicKey::from_slice(pubkey).is_err() {
                    return Some(Flaw::PubkeyInvalid);
                }
            }
        }

        None
    }
}

// Add governance-specific call types to CallType enum in message.rs
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ProposalCreation {
    pub title: String,
    pub description: String,
    pub vote_type: VoteType,
    pub actions: Vec<ProposalAction>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ProposalAction {
    Transfer {
        recipient: BitcoinAddress,
        amount: U128,
        asset: BlockTxTuple,
    },
    UpdateGovernanceSettings {
        new_settings: GovernanceSettings,
    },
    UpdateProposalSettings {
        new_settings: ProposalSettings,
    },
    UpdateExecutionSettings {
        new_settings: ExecutionSettings,
    },
    Custom {
        action_type: String,
        parameters: serde_json::Value,
    },
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Vote {
    pub proposal_id: BlockTxTuple,
    pub vote_cast: VoteCast,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum VoteCast {
    Simple {
        choice: u32,
    },
    Ranked {
        rankings: Vec<u32>,
    },
    Weighted {
        weights: Vec<U128>,
    },
} 