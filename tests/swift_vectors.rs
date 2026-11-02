use lattice_sdk::{
    primitives::{TransactionBody, crypto},
    transactions::{SigningAccount, encode_signed_transaction},
};
use std::collections::BTreeMap;
#[test]
fn actual_swift_keys_signatures_and_transaction_content_match() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("swift-vectors.json")).unwrap();
    assert_eq!(rows.len(), 80);
    for row in rows {
        let signer =
            SigningAccount::from_private_key(&[row["seed"].as_u64().unwrap() as u8; 32]).unwrap();
        let want = &row["swift"];
        assert_eq!(signer.address(), want["address"].as_str().unwrap());
        assert_eq!(signer.public_key(), want["publicKey"].as_str().unwrap());
        let mut wire = row["payload"]["transaction"]["body"].clone();
        wire["nonce"] = wire["nonce"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            .into();
        for (field, keys) in [
            ("accountActions", vec!["delta"]),
            (
                "depositActions",
                vec!["nonce", "amountDemanded", "amountDeposited"],
            ),
            ("receiptActions", vec!["nonce", "amountDemanded"]),
            (
                "withdrawalActions",
                vec!["nonce", "amountDemanded", "amountWithdrawn"],
            ),
        ] {
            for a in wire[field].as_array_mut().unwrap() {
                for k in &keys {
                    a[*k] = if *k == "delta" {
                        a[*k].as_str().unwrap().parse::<i64>().unwrap().into()
                    } else {
                        a[*k].as_str().unwrap().parse::<u64>().unwrap().into()
                    };
                }
            }
        }
        let body: TransactionBody = serde_json::from_value(wire).unwrap();
        let signed = signer.sign_body(&body).unwrap();
        assert_eq!(signed.body_cid, want["bodyCID"]);
        let preimage = body.signing_preimage().unwrap();
        assert_eq!(preimage, want["preimage"]);
        let signature = want["signature"].as_str().unwrap();
        assert!(crypto::verify(&preimage, signature, signer.public_key()));
        assert_eq!(want["rustSignatureAccepted"], "true");
        let encoded = encode_signed_transaction(
            &BTreeMap::from([(signer.public_key().into(), signature.into())]),
            &signed.body_cid,
        )
        .unwrap();
        assert_eq!(encoded.cid, want["transactionCID"]);
    }
}
