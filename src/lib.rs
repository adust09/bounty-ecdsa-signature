#![feature(iter_array_chunks)]
#![feature(result_option_inspect)]
#![allow(unused_imports)]

use std::sync::RwLock;

use lazy_static::lazy_static;
use tfhe::integer::ClientKey;

pub mod ecdsa;
pub mod field;
pub mod helper;
pub mod ops;
pub mod stats;

lazy_static! {
    pub static ref CLIENT_KEY: RwLock<Option<ClientKey>> = RwLock::new(None);
}
