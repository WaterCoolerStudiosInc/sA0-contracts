#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod staking {
    use ink::ToAccountId;
    use ink::{contract_ref, env::call};
    use ink::{
        env::{debug_println, DefaultEnvironment},
        prelude::{string::String, vec::Vec},
        storage::Mapping,
    };
    use psp22::{PSP22Error, PSP22};
    use psp34::{Id, PSP34Error, PSP34};

    use governance_nft::GovernanceNFTRef;

   

    pub const DAY: u64 = 86400 * 1000;
    pub const WITHDRAW_DELAY: u64 = 14 * DAY;
    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum StakingError {
        Invalid,
        Unauthorized,
        InvalidTimeWindow,
        NFTError(PSP34Error),
        TokenError(PSP22Error),
    }
    #[ink(storage)]
    pub struct Staking {
        creation_time: u64,
        reward_token_balance: u128,
        staked_token_balance: u128,
        rewards_per_second: u128,
        reward_stake_accumulation: u128,
        accumulated_rewards:u128,
        lst_accumulation_update: u64,
        owner: AccountId,
        governance_token: AccountId,
        nft: GovernanceNFTRef,
        governance_nfts: Mapping<AccountId, Vec<u128>>,
        unstake_requests: Mapping<u128, UnstakeRequest>,
        last_reward_claim:Mapping<u128,u64>
    }
    #[derive(Debug, PartialEq, Eq, Clone, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    struct UnstakeRequest {
        pub time: u64,
        pub token_value: u128,
        pub owner:AccountId
    }
    impl Staking {
        fn transfer_psp22_from(
            &self,
            from: &AccountId,
            to: &AccountId,
            amount: Balance,
        ) -> Result<(), StakingError> {
            let mut token: contract_ref!(PSP22) = self.governance_token.into();
            if let Err(e) = token.transfer_from(*from, *to, amount, Vec::new()) {
                return Err(StakingError::TokenError(e));
            }
            Ok(())
        }
        fn burn_psp34(&mut self, from: AccountId, id: u128) -> Result<(), StakingError> {
            if let Err(e) = self.nft.burn(from, id) {
                return Err(StakingError::NFTError(e));
            }
            Ok(())
        }
        fn mint_psp34(&mut self, to: AccountId, weight: u128) -> Result<(), StakingError> {
            if let Err(e) = self.nft.mint(to, weight) {
                return Err(StakingError::NFTError(e));
            }
            Ok(())
        }
        fn update_stake_accumulation(&mut self, curr_time: u64) -> Result<(), StakingError> {
            self.accumulated_rewards+=((curr_time - self.lst_accumulation_update) as u128)*self.rewards_per_second;
            self.reward_stake_accumulation +=
                self.staked_token_balance*((curr_time - self.lst_accumulation_update) as u128);
            self.lst_accumulation_update=curr_time;
            Ok(())
        }
        fn calculate_reward_share(&mut self,curr_time: u64,last_update:u64,stake_balance:u128)-> u128{
            let user_stake_weight=stake_balance*((curr_time-last_update) as u128);
            (self.accumulated_rewards*user_stake_weight)/self.reward_stake_accumulation
        }

        #[ink(constructor)]
        pub fn new(
            governance_token: AccountId,
            governance_nft_hash: Hash,
            interest_rate: u128,
        ) -> Self {
            use ink::{storage::Mapping, ToAccountId};

            let caller = Self::env().caller();
            let now = Self::env().block_timestamp();

            let nft_ref = GovernanceNFTRef::new(Self::env().account_id())
                .endowment(0)
                .code_hash(governance_nft_hash)
                .salt_bytes(&[9_u8.to_le_bytes().as_ref(), caller.as_ref()].concat()[..4])
                .instantiate();

            Self {
                creation_time: now,
                reward_token_balance: 0_u128,
                staked_token_balance: 0_u128,
                rewards_per_second: interest_rate,
                reward_stake_accumulation: 0,
                accumulated_rewards:0,
                lst_accumulation_update: now,
                owner: caller,
                governance_token: governance_token,
                nft: nft_ref,
                governance_nfts: Mapping::new(),
                unstake_requests: Mapping::new(),
                last_reward_claim:Mapping::new()
            }
        }
        #[ink(message)]
        pub fn get_governance_nft(&self) -> AccountId {
            GovernanceNFTRef::to_account_id(&self.nft)
        }
        #[ink(message)]
        pub fn wrap_tokens(
            &mut self,
            token_value: u128,
            to: Option<AccountId>,
        ) -> Result<(), StakingError> {
            let caller = Self::env().caller();
            let now = Self::env().block_timestamp();
            self.transfer_psp22_from(&caller, &Self::env().account_id(), token_value)?;
            self.update_stake_accumulation(now)?;
            self.staked_token_balance+=token_value;
            
           
            
            if to.is_some() {
                self.mint_psp34(to.unwrap(), token_value)?;
            } else {
                self.mint_psp34(caller, token_value)?;
            }

            Ok(())
        }

        #[ink(message)]
        pub fn add_token_value(
            &mut self,
            token_value: u128,
            nft_id: u128,
        ) -> Result<(), StakingError> {
            let caller = Self::env().caller();
            let now = Self::env().block_timestamp();
            self.transfer_psp22_from(&caller, &Self::env().account_id(), token_value)?;
            self.update_stake_accumulation(now)?;
            self.staked_token_balance+=token_value;
            if let Err(e) = self.nft.increment_weight(nft_id, token_value) {
                return Err(StakingError::NFTError(e));
            }
            Ok(())
        }
        #[ink(message)]
        pub fn remove_token_value(
            &mut self,
            token_value: u128,
            nft_id: u128,
        ) -> Result<(), StakingError> {
            let caller = Self::env().caller();
            let now = Self::env().block_timestamp();
            if let Err(e) = self.nft.decrement_weight(nft_id, token_value) {
                return Err(StakingError::NFTError(e));
            }
            self.update_stake_accumulation(now)?;
            self.transfer_psp22_from(&Self::env().account_id(), &caller, token_value)?;
            
            self.staked_token_balance-=token_value;
            Ok(())
        }
        #[ink(message)]
        pub fn claim_staking_rewards(&mut self,token_id: u128)->Result<(), StakingError> {
            let now = Self::env().block_timestamp();
            
            self.update_stake_accumulation(now)?;

            let data = self.nft.get_governance_data(token_id);
            let last_claim= self.last_reward_claim.get(token_id).unwrap_or(data.block_created);
            let reward= self.calculate_reward_share(now,last_claim,data.vote_weight);
            self.last_reward_claim.insert(token_id,&now);
            if let Err(e) = self.nft.increment_weight(token_id,reward) {
                return Err(StakingError::NFTError(e));
            }
            Ok(())
        }
        #[ink(message)]
        pub fn create_unwrap_request(&mut self, token_id: u128) -> Result<(), StakingError> {
            let now = Self::env().block_timestamp();
            let caller = Self::env().caller();
            let data = self.nft.get_governance_data(token_id);
            
            self.update_stake_accumulation(now);
             
            let last_claim= self.last_reward_claim.get(token_id).unwrap_or(data.block_created);
            let reward= self.calculate_reward_share(now,last_claim,data.vote_weight);
           
            self.staked_token_balance-=data.vote_weight;
            self.unstake_requests
                .insert(token_id, &UnstakeRequest{time:now, token_value:data.vote_weight+reward,owner:caller});
            self.burn_psp34(caller, token_id)?;
            Ok(())
        }
        #[ink(message)]
        pub fn unwrap(&mut self, token_id: u128) -> Result<(), StakingError> {
            let now = Self::env().block_timestamp();
            let caller = Self::env().caller();
            let data=  self.unstake_requests.get(token_id).unwrap();
            if now< data.time + WITHDRAW_DELAY{
                return Err(StakingError::InvalidTimeWindow);
            }
            if data.owner!= caller {
                return Err(StakingError::Unauthorized);
            }
            self.transfer_psp22_from(&Self::env().account_id(), &caller, data.token_value)?;
            self.unstake_requests.remove(token_id);
            Ok(())
        }
    }
}
