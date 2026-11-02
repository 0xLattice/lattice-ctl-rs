//! Exact pinned Swift NexusGenesis bootstrap for isolated mixed-runtime tests.
use lattice_cashew::{Header, Volume};
use lattice_sdk::primitives::*;
use lattice_validation::*;
use std::collections::BTreeMap;
pub fn volumes() -> anyhow::Result<Vec<lattice_cashew::SerializedVolume>> {
    let spec = ChainSpec {
        max_number_of_transactions_per_block: 5000,
        max_state_growth: 3_000_000,
        max_block_size: 1_000_000,
        premine: 175_320,
        target_block_time: 3_600_000,
        initial_reward: 1_048_576,
        halving_interval: 876_600,
        half_life: 120,
        wasm_policies: vec![],
    };
    let owner = crypto::create_address(
        "ed01fe416588df6e7fa5213c0d3e430f504bb5203172120c86b874826b55f53bdb7d",
    )
    .unwrap();
    let body = TransactionBody {
        account_actions: vec![AccountAction {
            owner,
            delta: spec.premine_amount().try_into().unwrap(),
        }],
        actions: vec![],
        deposit_actions: vec![],
        receipt_actions: vec![],
        withdrawal_actions: vec![],
        signers: vec![],
        nonce: 0,
        chain_path: vec!["Nexus".into()],
    };
    let tx = Transaction {
        signatures: BTreeMap::new(),
        body: Header::new(body).unwrap(),
    };
    let source = BTreeMap::<String, Vec<u8>>::new();
    let result = GenesisTemplate {
        spec: &spec,
        transactions: &[tx],
        children: &BTreeMap::new(),
        parent_state: None,
        timestamp: 1791094799000,
        target: UInt256([255; 32]),
        nonce: 0,
        version: 1,
    }
    .build(&source)
    .unwrap();
    let mut built = result.block;
    built.prev_state = Volume::new(LatticeState::empty().unwrap()).unwrap();
    built.parent_state = built.prev_state.clone();
    let block = Volume::new(built).unwrap();
    assert_eq!(
        block.0.raw_cid,
        "bafyreiggtg4ezifboyekbf4gxst2jr3mpjqcxsbmopy7w6fp46g4ngpgxa"
    );
    let volumes = lattice_cashew::volume::serialize_recursive(&block, &()).unwrap();
    Ok(volumes)
}
