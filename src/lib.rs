#![feature(iter_array_chunks)]
#![feature(result_option_inspect)]
#![allow(unused_imports)]

use std::sync::RwLock;

use ctor::ctor;
use lazy_static::lazy_static;
use logging_timer::Level;
use tfhe::integer::ClientKey;

pub mod ecdsa;
pub mod field;
pub mod helper;
pub mod numeral;
pub mod ops;
pub mod stats;

lazy_static! {
    pub static ref CLIENT_KEY: RwLock<Option<ClientKey>> = RwLock::new(None);
}

#[ctor]
fn init() {
    env_logger::builder()
        .filter_level(Level::Debug.to_level_filter())
        .parse_default_env()
        .init();
}
