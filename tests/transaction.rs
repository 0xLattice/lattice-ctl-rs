use lattice_ctl::transaction::*;
use lattice_sdk::transactions::SigningAccount;
#[test]
fn rejects_invalid_action_boundaries_before_signing() {
    let account = SigningAccount::from_private_key(&[7; 32]).unwrap();
    for amount in [0, i64::MAX as u64 + 1] {
        assert!(
            compose(
                &account,
                ActionPlan::Send {
                    to: account.address().into(),
                    amount
                },
                "Nexus",
                0,
                0
            )
            .is_err()
        );
    }
    assert!(chain_path("Nexus//Alpha").is_err());
    assert!(chain_path("Other").is_err());
    assert!(
        compose(
            &account,
            ActionPlan::Receipt {
                nonce: 0,
                demand: 1,
                demander: account.address().into(),
                directory: "Alpha/Beta".into()
            },
            "Nexus",
            0,
            0
        )
        .is_err()
    );
    assert!(
        compose(
            &account,
            ActionPlan::CommitChild {
                directory: "Alpha".into(),
                block: "bad".into(),
                parent_state: account.address().into(),
                previous: None
            },
            "Nexus",
            0,
            0
        )
        .is_err()
    );
}
