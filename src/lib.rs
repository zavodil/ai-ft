use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider,
};
use near_contract_standards::fungible_token::{
    FungibleToken, FungibleTokenCore, FungibleTokenResolver,
};
use near_contract_standards::storage_management::{
    StorageBalance, StorageBalanceBounds, StorageManagement,
};
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LazyOption, LookupMap};
use near_sdk::json_types::U128;
use near_sdk::{
    env, log, near_bindgen,
    serde::{Deserialize, Serialize},
    AccountId, BorshStorageKey, GasWeight, NearToken, PanicOnDefault, PromiseOrValue,
};
use near_sdk::{near, Gas};
use schemars::JsonSchema;

use std::convert::TryInto;

mod events;

pub type Balance = u128;

#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    Ft,
    FtMeta,
    Requests,
    Responses,
}

const MIN_RESPONSE_GAS: Gas = Gas::from_tgas(5);
const DATA_ID_REGISTER: u64 = 0;

pub type CryptoHash = [u8; 32];

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, JsonSchema, Clone)]
#[borsh(crate = "near_sdk::borsh")]
#[serde(crate = "near_sdk::serde")]
pub struct Request {
    data_id: CryptoHash,
    amount: u128,
    #[schemars(with = "String")]
    sender_id: AccountId,
    #[schemars(with = "String")]
    receiver_id: AccountId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, JsonSchema, Clone)]
#[borsh(crate = "near_sdk::borsh")]
#[serde(crate = "near_sdk::serde")]
pub struct Response {
    pub ok: bool,
    pub data: Option<String>,
    pub signature: Option<String>,
}

#[derive(Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Serialize))]
#[serde(crate = "near_sdk::serde")]
pub struct ResponseMsg {
    message: String,
    winner: AccountId,
}

pub type RequestId = u64;

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Contract {
    agent_name: String,
    operator_id: AccountId,

    ft: FungibleToken,

    metadata: LazyOption<FungibleTokenMetadata>,

    requests: LookupMap<RequestId, Request>,
    responses: LookupMap<RequestId, Response>,
    num_requests: u64,
}

#[near_bindgen]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.get().unwrap()
    }
}

#[near_bindgen]
impl FungibleTokenCore for Contract {
    #[payable]
    #[allow(unused_variables)]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        let receiver_balance = self.ft.ft_balance_of(receiver_id.clone()).0;
        if receiver_id == env::current_account_id() {
            panic!("Oh, look at you, trying to raid the Main Vault. Nice try, but that's against the rules. Try attacking something else, genius.")
        }
        if receiver_balance < amount.0 {
            self.register_receiver(&receiver_id);
            self.ft.ft_transfer(receiver_id, amount, memo);
        } else {
            let request_id: RequestId = self.num_requests;
            let sender_id = env::predecessor_account_id();

            let yield_promise = env::promise_yield_create(
                "await_response",
                &serde_json::to_vec(&(request_id,)).unwrap(),
                MIN_RESPONSE_GAS,
                GasWeight(0),
                DATA_ID_REGISTER,
            );

            let data_id: CryptoHash = env::read_register(DATA_ID_REGISTER)
                .expect("")
                .try_into()
                .expect("");

            let request_with_data_id = Request {
                data_id,
                amount: amount.0,
                sender_id: sender_id.clone(),
                receiver_id: receiver_id.clone(),
            };

            self.requests.insert(&request_id, &request_with_data_id);
            self.num_requests += 1;

            let message: String = format!(
                "{{\"sender_id\": \"{}\", \"receiver_id\": \"{}\"}}",
                sender_id.clone(),
                receiver_id.clone()
            );
            events::emit::run_agent(&self.agent_name, &message, Some(request_id));

            env::promise_return(yield_promise);
        }
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.ft.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        U128::from(self.ft.total_supply)
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.ft.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for Contract {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        let (used_amount, burned_amount) =
            self.ft
                .internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        if burned_amount > 0 {
            log!("Account @{} burned {}", sender_id, burned_amount);
        }
        used_amount.into()
    }
}

#[near]
impl StorageManagement for Contract {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        self.ft.storage_deposit(account_id, registration_only)
    }

    #[payable]
    #[allow(unused_variables)]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        env::panic_str("Not available")
    }

    #[payable]
    #[allow(unused_variables)]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        env::panic_str("Not available")
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.ft.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.ft.storage_balance_of(account_id)
    }
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(
        total_supply: U128,
        metadata: FungibleTokenMetadata,
        agent_name: String,
        operator_id: AccountId,
    ) -> Self {
        metadata.assert_valid();
        let mut ft = FungibleToken::new(StorageKey::Ft);
        ft.internal_register_account(&env::current_account_id());
        ft.internal_deposit(&env::current_account_id(), total_supply.into());

        near_contract_standards::fungible_token::events::FtMint {
            owner_id: &env::current_account_id(),
            amount: total_supply,
            memo: Some("Initial tokens supply is minted"),
        }
        .emit();

        Self {
            operator_id,
            agent_name,
            ft,
            metadata: LazyOption::new(StorageKey::FtMeta, Some(&metadata)),

            requests: LookupMap::new(StorageKey::Requests),
            responses: LookupMap::new(StorageKey::Responses),
            num_requests: 0,
        }
    }

    pub fn respond(&mut self, data_id: CryptoHash, request_id: RequestId, response: Response) {
        self.assert_operator();

        if self.requests.get(&request_id).is_none() {
            panic!("Request ID not found");
        }

        self.responses.insert(&request_id, &response);

        env::promise_yield_resume(&data_id, &serde_json::to_vec(&(request_id,)).unwrap());
    }

    #[private]
    pub fn await_response(&mut self, request_id: RequestId) -> PromiseOrValue<Response> {
        let response: Option<Response> = self.responses.get(&request_id);
        if let Some(response) = response {
            self.responses.remove(&request_id);

            let request = self.requests.remove(&request_id).expect("Wrong request");

            let response_text = response.data.clone().unwrap_or_default();

            let parsed_message = serde_json::from_str::<ResponseMsg>(&response_text)
                .expect("Wrong response message format");

            if response.ok {
                if parsed_message.winner == request.receiver_id {
                    self.register_receiver(&request.receiver_id);

                    self.ft.internal_transfer(
                        &request.sender_id,
                        &request.receiver_id,
                        request.amount,
                        Some(parsed_message.message),
                    );
                } else if parsed_message.winner == request.sender_id {
                    self.ft.internal_transfer(
                        &request.receiver_id,
                        &request.sender_id,
                        request.amount,
                        Some(parsed_message.message),
                    );
                } else {
                    panic!("Unknown response received");
                }
            }

            PromiseOrValue::Value(response)
        } else {
            panic!("Response is missing for {}", request_id);
        }
    }

    #[private]
    pub fn set_ft_metadata(&mut self, metadata: FungibleTokenMetadata) {
        metadata.assert_valid();
        self.metadata.set(&metadata);
    }

    #[private]
    pub fn set_operator_id(&mut self, operator_id: AccountId) {
        self.operator_id = operator_id;
    }

    #[private]
    pub fn set_agent_name(&mut self, agent_name: String) {
        self.agent_name = agent_name;
    }

    pub fn get_request(&self, request_id: RequestId) -> Request {
        self.requests.get(&request_id).unwrap()
    }

    #[private]
    pub fn remove_request(&mut self, request_id: RequestId) {
        self.assert_operator();
        self.requests.remove(&request_id);
        self.responses.remove(&request_id);
    }
}

impl Contract {
    fn register_receiver(&mut self, receiver_id: &AccountId) {
        // register new account if needed
        if !self.ft.accounts.contains_key(receiver_id) {
            self.ft.internal_register_account(receiver_id);
        }
    }
    fn assert_operator(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.operator_id,
            "ERR_NOT_AN_OPERATOR"
        );
    }
}
