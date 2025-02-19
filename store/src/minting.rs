use mintbase_deps::common::{
    NewSplitOwner,
    Royalty,
    RoyaltyArgs,
    SplitBetweenUnparsed,
    SplitOwners,
    TokenMetadata,
};
use mintbase_deps::constants::MAX_LEN_PAYOUT;
use mintbase_deps::logging::{
    log_grant_minter,
    log_nft_batch_mint,
    log_revoke_minter,
};
use mintbase_deps::near_sdk::{
    self,
    env,
    near_bindgen,
    AccountId,
    Balance,
};
use mintbase_deps::token::Token;

use crate::*;

#[near_bindgen]
impl MintbaseStore {
    // -------------------------- change methods ---------------------------

    /// The core `Store` function. `mint_token` mints `num_to_mint` copies of
    /// a token.
    ///
    /// Restrictions:
    /// - Only minters may call this function.
    /// - `owner_id` must be a valid Near address.
    /// - Because of logging limits, this method may mint at most 99 tokens per call.
    /// - 1.0 >= `royalty_f` >= 0.0. `royalty_f` is ignored if `royalty` is `None`.
    /// - If a `royalty` is provided, percentages **must** be non-negative and add to one.
    /// - The maximum length of the royalty mapping is 50.
    ///
    /// This method is the most significant increase of storage costs on this
    /// contract. Minters are expected to manage their own storage costs.
    #[payable]
    pub fn nft_batch_mint(
        &mut self,
        owner_id: AccountId,
        metadata: TokenMetadata,
        num_to_mint: u64,
        royalty_args: Option<RoyaltyArgs>,
        split_owners: Option<SplitBetweenUnparsed>,
    ) {
        assert!(num_to_mint > 0);
        assert!(num_to_mint <= 125); // upper gas limit
        assert!(env::attached_deposit() >= 1);
        let minter_id = env::predecessor_account_id();
        assert!(
            self.minters.contains(&minter_id),
            "{} not a minter",
            minter_id.as_ref()
        );

        // Calculating storage consuption upfront saves gas if the transaction
        // were to fail later.
        let covered_storage = env::account_balance()
            - (env::storage_usage() as u128 * self.storage_costs.storage_price_per_byte);
        let (metadata, md_size) = TokenMetadata::from_with_size(metadata, num_to_mint);
        let roy_len = royalty_args
            .as_ref()
            .map(|pre_roy| {
                let len = pre_roy.split_between.len();
                len as u32
            })
            .unwrap_or(0);
        let split_len = split_owners
            .as_ref()
            .map(|pre_split| {
                let len = pre_split.len();
                len as u32
            })
            // if there is no split map, there still is an owner, thus default to 1
            .unwrap_or(1);
        assert!(roy_len + split_len <= MAX_LEN_PAYOUT);
        let expected_storage_consumption: Balance =
            self.storage_cost_to_mint(num_to_mint, md_size, roy_len, split_len);
        assert!(
            covered_storage >= expected_storage_consumption,
            "covered: {}; need: {}",
            covered_storage,
            expected_storage_consumption
        );

        let checked_royalty = royalty_args.map(Royalty::new);
        let checked_split = split_owners.map(SplitOwners::new);

        let mut owned_set = self.get_or_make_new_owner_set(&owner_id);

        // Lookup Id is used by the token to lookup Royalty and Metadata fields on
        // the contract (to avoid unnecessary duplication)
        let lookup_id: u64 = self.tokens_minted;
        let royalty_id = checked_royalty.clone().map(|royalty| {
            self.token_royalty
                .insert(&lookup_id, &(num_to_mint as u16, royalty));
            lookup_id
        });

        let meta_ref = metadata.reference.as_ref().map(|s| s.to_string());
        let meta_extra = metadata.extra.as_ref().map(|s| s.to_string());
        self.token_metadata
            .insert(&lookup_id, &(num_to_mint as u16, metadata));

        // Mint em up hot n fresh with a side of vegan bacon
        (0..num_to_mint).for_each(|i| {
            let token_id = self.tokens_minted + i;
            let token = Token::new(
                owner_id.clone(),
                token_id,
                lookup_id,
                royalty_id,
                checked_split.clone(),
                minter_id.clone(),
            );
            owned_set.insert(&token_id);
            self.tokens.insert(&token_id, &token);
        });
        self.tokens_minted += num_to_mint;
        self.tokens_per_owner.insert(&owner_id, &owned_set);

        let minted = self.tokens_minted;
        log_nft_batch_mint(
            minted - num_to_mint,
            minted - 1,
            minter_id.as_ref(),
            owner_id.as_ref(),
            &checked_royalty,
            &checked_split,
            &meta_ref,
            &meta_extra,
        );
    }

    /// Modify the minting privileges of `account_id`. Minters are able to
    /// mint tokens on this `Store`.
    ///
    /// Only the store owner may call this function.
    ///
    /// This method increases storage costs of the contract.
    #[payable]
    pub fn grant_minter(
        &mut self,
        account_id: AccountId,
    ) {
        self.assert_store_owner();
        let account_id: AccountId = account_id;
        // does nothing if account_id is already a minter
        if self.minters.insert(&account_id) {
            log_grant_minter(&account_id);
        }
    }

    /// Modify the minting privileges of `account_id`. Minters are able to
    /// mint tokens on this `Store`. The current `Store` owner cannot revoke
    /// themselves.
    ///
    /// Only the store owner may call this function.
    #[payable]
    pub fn revoke_minter(
        &mut self,
        account_id: AccountId,
    ) {
        self.assert_store_owner();
        assert_ne!(account_id, self.owner_id, "can't revoke owner");
        if !self.minters.remove(&account_id) {
            env::panic_str("not a minter")
        } else {
            log_revoke_minter(&account_id);
        }
    }

    // -------------------------- view methods -----------------------------

    /// Check if `account_id` is a minter.
    pub fn check_is_minter(
        &self,
        account_id: AccountId,
    ) -> bool {
        self.minters.contains(&account_id)
    }

    /// Lists all account IDs that are currently allowed to mint on this
    /// contract.
    pub fn list_minters(&self) -> Vec<AccountId> {
        self.minters.iter().collect()
    }

    // -------------------------- private methods --------------------------
    // -------------------------- internal methods -------------------------

    /// Get the storage in bytes to mint `num_tokens` each with
    /// `metadata_storage` and `len_map` royalty receivers.
    /// Internal
    fn storage_cost_to_mint(
        &self,
        num_tokens: u64,
        metadata_storage: StorageUsage,
        num_royalties: u32,
        num_splits: u32,
    ) -> near_sdk::Balance {
        // create an entry in tokens_per_owner
        self.storage_costs.common
            // create a metadata record
            + metadata_storage as u128 * self.storage_costs.storage_price_per_byte
            // create a royalty record
            + num_royalties as u128 * self.storage_costs.common
            // create n tokens each with splits stored on-token
            + num_tokens as u128 * (self.storage_costs.token + num_splits as u128 * self.storage_costs.common)
    }
}
