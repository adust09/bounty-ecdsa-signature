use fhe::{
    ecdsa::{ecdsa_sign, ecdsa_sign_native},
    helper::{set_client_key, u256_from_decimal_string},
    numeral::Numeral,
    ops::secp256k1::prelude::*,
    stats::ProtocolStats,
};
use tfhe::{
    integer::{keycache::IntegerKeyCache, U256},
    shortint::prelude::PARAM_MESSAGE_2_CARRY_2,
};

fn main() {
    const NUM_BLOCK: usize = 128;
    let (client_key, server_key) = IntegerKeyCache.get_from_params(PARAM_MESSAGE_2_CARRY_2);
    set_client_key(&client_key);

    let sk = u256_from_decimal_string(
        "32670510020758816978083085130507043184471273380659243275938904335757337482424",
    );
    let nonce = u256_from_decimal_string(
        "158972629851468960855479098042189567798917817837573660423710583832714848",
    );
    let msg = u256_from_decimal_string(
        "65108744961846543415519418389643270459525907322081164366671650776835723265410",
    );

    let signature_nativce = ecdsa_sign_native(sk, nonce, msg, *GENERATOR, *FQ_MODULO, *FR_MODULO);
    println!(
        "Native signature r: {}, s: {}",
        signature_nativce.0.format(),
        signature_nativce.1.format()
    );

    let sk_enc = client_key.encrypt_radix(sk, NUM_BLOCK);
    let nonce_enc = client_key.encrypt_radix(nonce, NUM_BLOCK);

    let signature = ecdsa_sign::<NUM_BLOCK, _>(
        &sk_enc,
        &nonce_enc,
        msg,
        *GENERATOR,
        *FQ_MODULO,
        *FR_MODULO,
        &server_key,
    );

    println!(
        "signature: r: {}, s: {}",
        U256::decrypt(&signature.0, &client_key).format(),
        U256::decrypt(&signature.1, &client_key).format()
    );
    println!("stats: {}", ProtocolStats::stats());
}
