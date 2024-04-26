/// Optimism-specific genesis fields.
use alloy_genesis::Genesis;
use reth_primitives::{serde_helper::u64_opt_via_ruint, ChainSpec, ForkCondition, Hardfork};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

/// Genesis type for Optimism networks.
#[derive(Default, Debug)]
pub struct OptimismGenesis {
    /// Wraps Eth genesis fields.
    pub eth_genesis: Genesis,

    /// Config field.
    pub optimism_config: OptimismConfig,
}

/// Optimism config.
#[derive(Default, Deserialize, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct OptimismConfig {
    /// Bedrock switch block (None = no fork, 0 = already on bedrock).
    #[serde(deserialize_with = "u64_opt_via_ruint::deserialize")]
    pub bedrock_block: Option<u64>,

    /// Regolith switch time (None = no fork, 0 = already on regolith).
    #[serde(deserialize_with = "u64_opt_via_ruint::deserialize")]
    pub regolith_timestamp: Option<u64>,

    /// Ecotone switch time (None = no fork, 0 = already on ecotone).
    #[serde(deserialize_with = "u64_opt_via_ruint::deserialize")]
    pub ecotone_timestamp: Option<u64>,

    /// Canyon switch time (None = no fork, 0 = already on ecotone).
    #[serde(deserialize_with = "u64_opt_via_ruint::deserialize")]
    pub canyon_timestamp: Option<u64>,

    /// Optimism object
    pub optimism: Option<OptimismObject>,
}

/// Optimism object, includes additional EIP related information.
#[derive(Default, Deserialize, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct OptimismObject {
    /// EIP-1559 elasticity.
    pub eip1559_elasticity: u64,

    /// EIP-1559 denominator
    pub eip1559_denominator: u64,

    /// EIP-1559 Canyon denominator
    pub eip1559_denominator_canyon: u64,
}

impl<'de> Deserialize<'de> for OptimismGenesis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        let optimsim_genesis = raw.clone();
        let config = raw
            .get("config")
            .ok_or_else(|| serde::de::Error::custom("config field missing"))?
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("config should be an object"))?;

        let eth_genesis: Genesis =
            Genesis::deserialize(optimsim_genesis).map_err(serde::de::Error::custom)?;
        let optimism_config: OptimismConfig =
            serde_json::from_value(json!(config)).map_err(serde::de::Error::custom)?;

        Ok(OptimismGenesis { eth_genesis, optimism_config })
    }
}

impl From<OptimismGenesis> for ChainSpec {
    fn from(optimsim_genesis: OptimismGenesis) -> ChainSpec {
        let mut chain_spec: ChainSpec = optimsim_genesis.eth_genesis.into();

        if let Some(block) = optimsim_genesis.optimism_config.bedrock_block {
            chain_spec.hardforks.insert(Hardfork::Bedrock, ForkCondition::Block(block));
        }
        if let Some(timestamp) = optimsim_genesis.optimism_config.regolith_timestamp {
            chain_spec.hardforks.insert(Hardfork::Regolith, ForkCondition::Timestamp(timestamp));
        }
        if let Some(timestamp) = optimsim_genesis.optimism_config.ecotone_timestamp {
            chain_spec.hardforks.insert(Hardfork::Ecotone, ForkCondition::Timestamp(timestamp));
        }
        if let Some(timestamp) = optimsim_genesis.optimism_config.canyon_timestamp {
            chain_spec.hardforks.insert(Hardfork::Canyon, ForkCondition::Timestamp(timestamp));
        }

        chain_spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_genesis() {
        let genesis = r#"
    {
      "nonce": 9,
      "config": {
        "chainId": 1,
        "bedrockBlock": 10,
        "regolithTime": 20,
        "ecotoneTime": 30,
        "canyonTime": 40,
        "optimism": {
          "eip1559Elasticity": 50,
          "eip1559Denominator": 60,
          "eip1559DenominatorCanyon": 70
        }
      }
    }
    "#;
        let optimism_genesis: OptimismGenesis = serde_json::from_str(genesis).unwrap();

        let actual_nonce = optimism_genesis.eth_genesis.nonce;
        assert_eq!(actual_nonce, 9);
        let actual_chain_id = optimism_genesis.eth_genesis.config.chain_id;
        assert_eq!(actual_chain_id, 1);

        let actual_bedrock_block = optimism_genesis.optimism_config.bedrock_block;
        assert_eq!(actual_bedrock_block, Some(10));
        let actual_regolith_timestamp = optimism_genesis.optimism_config.regolith_timestamp;
        assert_eq!(actual_regolith_timestamp, Some(20));
        let actual_ecotone_timestamp = optimism_genesis.optimism_config.ecotone_timestamp;
        assert_eq!(actual_ecotone_timestamp, Some(30));
        let actual_canyon_timestamp = optimism_genesis.optimism_config.canyon_timestamp;
        assert_eq!(actual_canyon_timestamp, Some(40));

        let optimism_object = optimism_genesis.optimism_config.optimism.unwrap();
        let actual_eip1559_elasticity = optimism_object.eip1559_elasticity;
        assert_eq!(actual_eip1559_elasticity, 50);
        let actual_eip1559_denominator = optimism_object.eip1559_denominator;
        assert_eq!(actual_eip1559_denominator, 60);
        let actual_eip1559_denominator_canyon = optimism_object.eip1559_denominator_canyon;
        assert_eq!(actual_eip1559_denominator_canyon, 70);
    }

    #[test]
    fn optimism_genesis_into_chainspec() {
        let optimism_genesis = OptimismGenesis {
            eth_genesis: Genesis::default(),
            optimism_config: OptimismConfig {
                bedrock_block: Some(1),
                regolith_timestamp: Some(2),
                ecotone_timestamp: Some(3),
                canyon_timestamp: Some(4),
                ..Default::default()
            },
        };

        let chain_spec: ChainSpec = optimism_genesis.into();

        assert!(chain_spec.is_fork_active_at_block(Hardfork::Bedrock, 1));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Regolith, 2));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Ecotone, 3));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Canyon, 4));
    }
}
