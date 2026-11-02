use anyhow::{Result, ensure};
use lattice_sdk::{
    primitives::*,
    transactions::{SigningAccount, transaction_payload},
};
use serde_json::Value;
use std::collections::BTreeMap;

pub enum ActionPlan {
    Send {
        to: String,
        amount: u64,
    },
    Deposit {
        nonce: u64,
        demand: u64,
        lock: u64,
    },
    Receipt {
        nonce: u64,
        demand: u64,
        demander: String,
        directory: String,
    },
    Withdraw {
        nonce: u64,
        demand: u64,
        demander: String,
        amount: u64,
        to: Option<String>,
    },
    CommitChild {
        directory: String,
        block: String,
        parent_state: String,
        previous: Option<String>,
    },
}
pub fn chain_path(chain: &str) -> Result<Vec<String>> {
    let path: Vec<String> = chain.split('/').map(str::to_owned).collect();
    ensure!(
        path.first().is_some_and(|p| p == "Nexus")
            && path
                .iter()
                .all(|p| !p.is_empty() && !p.chars().any(char::is_control)),
        "chain must be an absolute Nexus path with nonempty edges"
    );
    Ok(path)
}
fn address(value: &str) -> Result<()> {
    ensure!(standard_cid(value), "not a standard Lattice address");
    Ok(())
}
fn directory(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && !value.contains('/'),
        "directory must be one nonempty edge"
    );
    Ok(())
}
fn delta(value: u64) -> Result<i64> {
    ensure!(
        value > 0 && value <= i64::MAX as u64,
        "amount must be between 1 and Int64.max"
    );
    Ok(value as i64)
}

pub fn compose(
    signer: &SigningAccount,
    plan: ActionPlan,
    chain: &str,
    nonce: u64,
    fee: u64,
) -> Result<TransactionBody> {
    let owner = signer.address().to_owned();
    let mut body = TransactionBody {
        account_actions: vec![],
        actions: vec![],
        deposit_actions: vec![],
        receipt_actions: vec![],
        withdrawal_actions: vec![],
        signers: vec![owner.clone()],
        nonce,
        chain_path: chain_path(chain)?,
    };
    match plan {
        ActionPlan::Send { to, amount } => {
            address(&to)?;
            let n = delta(amount)?;
            body.account_actions.extend([
                AccountAction {
                    owner: owner.clone(),
                    delta: -n,
                },
                AccountAction {
                    owner: to,
                    delta: n,
                },
            ]);
        }
        ActionPlan::Deposit {
            nonce,
            demand,
            lock,
        } => {
            body.account_actions.push(AccountAction {
                owner: owner.clone(),
                delta: -delta(lock)?,
            });
            body.deposit_actions.push(DepositAction {
                nonce: nonce.into(),
                demander: owner.clone(),
                amount_demanded: demand,
                amount_deposited: lock,
            });
        }
        ActionPlan::Receipt {
            nonce,
            demand,
            demander,
            directory: edge,
        } => {
            address(&demander)?;
            directory(&edge)?;
            body.receipt_actions.push(ReceiptAction {
                withdrawer: owner.clone(),
                nonce: nonce.into(),
                demander,
                amount_demanded: demand,
                directory: edge,
            });
        }
        ActionPlan::Withdraw {
            nonce,
            demand,
            demander,
            amount,
            to,
        } => {
            address(&demander)?;
            let to = to.unwrap_or_else(|| owner.clone());
            address(&to)?;
            body.account_actions.push(AccountAction {
                owner: to,
                delta: delta(amount)?,
            });
            body.withdrawal_actions.push(WithdrawalAction {
                withdrawer: owner.clone(),
                nonce: nonce.into(),
                demander,
                amount_demanded: demand,
                amount_withdrawn: amount,
            });
        }
        ActionPlan::CommitChild {
            directory: edge,
            block,
            parent_state,
            previous,
        } => {
            directory(&edge)?;
            ensure!(
                standard_cid(&block) && standard_cid(&parent_state),
                "block and parent state must be standard CIDs"
            );
            body.actions.push(Action {
                key: format!("child/{edge}"),
                old_value: previous,
                new_value: Some(format!("{block}@{parent_state}")),
            });
        }
    }
    if fee > 0 {
        body.account_actions.push(AccountAction {
            owner,
            delta: -delta(fee)?,
        });
    }
    Ok(body)
}
pub fn signed(
    signer: &SigningAccount,
    body: &TransactionBody,
) -> Result<(BTreeMap<String, String>, Value)> {
    let signed = signer.sign_body(body)?;
    let signatures = BTreeMap::from([(signed.public_key, signed.signature)]);
    let payload = transaction_payload(&signatures, body)?;
    Ok((signatures, payload))
}
