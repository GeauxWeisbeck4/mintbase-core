use std::fmt;

use near_sdk::borsh::{
    self,
    BorshDeserialize,
    BorshSerialize,
};
use near_sdk::serde::{
    Deserialize,
    Serialize,
};
use near_sdk::AccountId;

// TODO: rename to `TokenOwner`
#[cfg_attr(feature = "wasm", derive(BorshDeserialize, BorshSerialize))]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum Owner {
    /// Standard pattern: owned by a user.
    Account(AccountId),
    /// Compose pattern: owned by a token on this contract.
    TokenId(u64),
    /// Cross-compose pattern: owned by a token on another contract.
    CrossKey(crate::common::TokenKey),
    /// Lock: temporarily locked until some callback returns.
    Lock(AccountId),
}

impl fmt::Display for Owner {
    fn fmt(
        &self,
        f: &mut fmt::Formatter,
    ) -> fmt::Result {
        match self {
            Owner::Account(s) => write!(f, "{}", s),
            Owner::TokenId(n) => write!(f, "{}", n),
            Owner::CrossKey(key) => write!(f, "{}", key),
            Owner::Lock(_) => panic!("locked"),
        }
    }
}
