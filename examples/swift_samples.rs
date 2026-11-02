use lattice_ctl::transaction::{ActionPlan, compose, signed};
use lattice_sdk::transactions::{SigningAccount, encode_signed_transaction};
use serde_json::json;
fn main() {
    let other = SigningAccount::from_private_key(&[42; 32]).unwrap();
    let mut rows = vec![];
    for seed in [0, 1, 7, 255] {
        for fee in [0, 7] {
            for (nonce, chain) in [(0, "Nexus"), (u64::MAX, "Nexus/Alpha")] {
                for kind in 0..5 {
                    let signer = SigningAccount::from_private_key(&[seed; 32]).unwrap();
                    let to = other.address().to_owned();
                    let plan = match kind {
                        0 => ActionPlan::Send {
                            to: to.clone(),
                            amount: 23,
                        },
                        1 => ActionPlan::Deposit {
                            nonce: 17,
                            demand: 11,
                            lock: 23,
                        },
                        2 => ActionPlan::Receipt {
                            nonce: 17,
                            demand: 11,
                            demander: to.clone(),
                            directory: "Alpha".into(),
                        },
                        3 => ActionPlan::Withdraw {
                            nonce: 17,
                            demand: 11,
                            demander: to.clone(),
                            amount: 23,
                            to: None,
                        },
                        _ => ActionPlan::CommitChild {
                            directory: "Alpha".into(),
                            block: to.clone(),
                            parent_state: signer.address().into(),
                            previous: Some("old@state".into()),
                        },
                    };
                    let body = compose(&signer, plan, chain, nonce, fee).unwrap();
                    let (signatures, payload) = signed(&signer, &body).unwrap();
                    let signed = signer.sign_body(&body).unwrap();
                    let transaction =
                        encode_signed_transaction(&signatures, &signed.body_cid).unwrap();
                    rows.push(json!({"seed":seed,"fee":fee,"nonce":nonce.to_string(),"chain":chain,"kind":kind,"address":signer.address(),"publicKey":signer.public_key(),"bodyCID":signed.body_cid,"signature":signed.signature,"preimage":body.signing_preimage().unwrap(),"transactionCID":transaction.cid,"payload":payload}));
                }
            }
        }
    }
    println!("{}", json!(rows));
}
