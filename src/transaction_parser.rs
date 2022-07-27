use std::{collections::HashMap, fmt::Debug, num::ParseIntError, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
pub use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::ParsePubkeyError;
pub use solana_sdk::{
    clock::UnixTimestamp,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Signature,
    slot_history::Slot,
};

pub use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransactionWithStatusMeta, UiInstruction,
    UiTransactionEncoding, UiTransactionTokenBalance
};

pub use crate::{
    instruction_parser::{BindInstructions, InstructionContext},
    log_parser::{self, ProgramContext, ProgramLog},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    SolanaClientResult(#[from] solana_client::client_error::ClientError),
    #[error(transparent)]
    LogParseError(#[from] crate::log_parser::Error),
    #[error("Field `meta` is empty in response of {0} tx request")]
    EmptyMetaInTransaction(Signature),
    #[error("Field `meta.log_messages` is empty in response of {0} tx request")]
    EmptyLogsInTransaction(Signature),
    #[error(transparent)]
    InstructionParsingError(#[from] crate::instruction_parser::Error),
    #[error(transparent)]
    ParsePubkeyError(#[from] ParsePubkeyError),
    #[error("Can't find ix ctx {0:?} in logs")]
    InstructionLogsConsistencyError(InstructionContext),
    #[error("Provided log and provided ix not match by owner")]
    InstructionLogsOwnerError { ix_owner: Pubkey, log_owner: Pubkey },
    #[error("Failed while transaction decoding with signature: {0}")]
    ErrorWhileDecodeTransaction(Signature),
    #[error(transparent)]
    ParseIntError(#[from] ParseIntError),
    #[error("Pre token account don't match with post")]
    WrongBalanceAccountConsistance(Pubkey),
}

#[async_trait]
pub trait BindTransactionLogs {
    async fn bind_transaction_logs(
        &self,
        signature: Signature,
    ) -> Result<HashMap<ProgramContext, Vec<ProgramLog>>, Error>;
}

#[async_trait]
impl BindTransactionLogs for RpcClient {
    async fn bind_transaction_logs(
        &self,
        signature: Signature,
    ) -> Result<HashMap<ProgramContext, Vec<ProgramLog>>, Error> {
        Ok(log_parser::parse_events(
            self.get_transaction(&signature, UiTransactionEncoding::Base58)
                .await?
                .transaction
                .meta
                .ok_or(Error::EmptyMetaInTransaction(signature))?
                .log_messages
                .ok_or(Error::EmptyLogsInTransaction(signature))?
                .as_slice(),
        )?)
    }
}

pub type AmountDiff = i128;
#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionParsedMeta {
    pub meta: HashMap<ProgramContext, (Instruction, Vec<ProgramLog>)>,
    pub slot: Slot,
    pub block_time: Option<UnixTimestamp>,
    pub lamports_changes: HashMap<Pubkey, AmountDiff>,
    pub token_balances_changes: HashMap<WalletContext, AmountDiff>,
}

#[cfg(feature = "anchor")]
mod anchor {
    use std::io;

    use crate::instruction_parser::AccountMeta;
    use anchor_lang::{AnchorDeserialize, Discriminator, Owner};

    use super::{ProgramLog, TransactionParsedMeta};

    impl TransactionParsedMeta {
        pub fn find_ix<I: Discriminator + Owner + AnchorDeserialize>(
            &self,
        ) -> Result<Vec<(I, &Vec<ProgramLog>, Vec<AccountMeta>)>, io::Error> {
            use crate::ParseInstruction;
            self.meta
                .iter()
                .filter_map(|(ctx, meta)| ctx.program_id.eq(&I::owner()).then(|| meta))
                .filter_map(|(ix, logs)| {
                    Some(
                        ix.parse_instruction::<I>()?
                            .map(|result_with_ix| (result_with_ix, logs, ix.accounts.clone())),
                    )
                })
                .collect::<Result<_, _>>()
        }
    }
}

#[async_trait]
pub trait BindTransactionInstructionLogs {
    async fn bind_transaction_instructions_logs(
        &self,
        signature: Signature,
    ) -> Result<TransactionParsedMeta, Error>;
}

#[async_trait]
impl BindTransactionInstructionLogs for RpcClient {
    async fn bind_transaction_instructions_logs(
        &self,
        signature: Signature,
    ) -> Result<TransactionParsedMeta, Error> {
        let EncodedConfirmedTransactionWithStatusMeta {
            transaction,
            slot,
            block_time,
        } = self
            .get_transaction(&signature, UiTransactionEncoding::Binary)
            .await?;
        let mut instructions = transaction.bind_instructions(signature)?;

        let meta = transaction
            .meta
            .as_ref()
            .ok_or(Error::EmptyMetaInTransaction(signature))?;

        Ok(TransactionParsedMeta {
            slot,
            block_time,
            meta: log_parser::parse_events(
                meta.log_messages
                    .as_ref()
                    .ok_or(Error::EmptyLogsInTransaction(signature))?
                    .as_slice(),
            )?
            .into_iter()
            .map(|(ctx, events)| {
                let ix_ctx = InstructionContext {
                    program_id: ctx.program_id,
                    call_index: ctx.call_index,
                };
                let (ix, outer_ix) = instructions
                    .remove(&ix_ctx)
                    .ok_or(Error::InstructionLogsConsistencyError(ix_ctx))?;

                // TODO Add validation of outer ix
                if (outer_ix.is_none() && ctx.invoke_level.get() == 1)
                    || (outer_ix.is_some() && ctx.invoke_level.get() != 1)
                {
                    Ok((ctx, (ix, events)))
                } else {
                    Err(Error::InstructionLogsConsistencyError(ix_ctx))
                }
            })
            .collect::<Result<_, Error>>()?,
            lamports_changes: transaction.get_lamports_changes(&signature)?,
            token_balances_changes: transaction.get_assets_changes(&signature)?,
        })
    }
}

pub trait GetLamportsChanges {
    fn get_lamports_changes(
        &self,
        signature: &Signature,
    ) -> Result<HashMap<Pubkey, AmountDiff>, Error>;
}
impl GetLamportsChanges for EncodedTransactionWithStatusMeta {
    fn get_lamports_changes(
        &self,
        signature: &Signature,
    ) -> Result<HashMap<Pubkey, AmountDiff>, Error> {
        let msg = self
            .transaction
            .decode()
            .ok_or(Error::ErrorWhileDecodeTransaction(*signature))?
            .message;
        let meta = self
            .meta
            .as_ref()
            .ok_or(Error::EmptyMetaInTransaction(*signature))?;

        Ok(meta
            .pre_balances
            .iter()
            .zip(meta.post_balances.iter())
            .enumerate()
            .map(|(index, (old_balance, new_balance))| {
                (index, *new_balance as i128 - *old_balance as i128)
            })
            .map(|(index, diff)| (msg.static_account_keys()[index], diff))
            .collect())
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WalletContext {
    pub wallet_address: Pubkey,
    pub wallet_owner: Option<Pubkey>,
    pub token_mint: Pubkey,
}
impl WalletContext {
    fn try_new(balance: &UiTransactionTokenBalance, accounts: &[Pubkey]) -> Result<Self, Error> {
        Ok(WalletContext {
            wallet_address: accounts[balance.account_index as usize],
            wallet_owner: balance.owner.as_deref().map(Pubkey::from_str).transpose()?,
            token_mint: Pubkey::from_str(balance.mint.as_str())?,
        })
    }
}

pub trait GetAssetsChanges {
    fn get_assets_changes(
        &self,
        signature: &Signature,
    ) -> Result<HashMap<WalletContext, AmountDiff>, Error>;
}
impl GetAssetsChanges for EncodedTransactionWithStatusMeta {
    fn get_assets_changes(
        &self,
        signature: &Signature,
    ) -> Result<HashMap<WalletContext, AmountDiff>, Error> {
        let msg = self
            .transaction
            .decode()
            .ok_or(Error::ErrorWhileDecodeTransaction(*signature))?
            .message;
        let meta = self
            .meta
            .as_ref()
            .ok_or(Error::EmptyMetaInTransaction(*signature))?;

        let try_parse_balance = |balance: &UiTransactionTokenBalance| {
            Ok((
                WalletContext::try_new(balance, msg.static_account_keys())?,
                balance.ui_token_amount.amount.parse()?,
            ))
        };

        meta.pre_token_balances
            .as_ref()
            .zip(meta.post_token_balances.as_ref())
            .map(|(pre_token_balances, post_token_balances)| {
                let balances_diff = post_token_balances
                    .iter()
                    .map(try_parse_balance)
                    .collect::<Result<HashMap<_, _>, Error>>()?;

                pre_token_balances.iter().map(try_parse_balance).try_fold(
                    balances_diff,
                    |mut balances_diff, result_with_ctx| {
                        let (wallet_ctx, pre_balance) = result_with_ctx?;

                        *balances_diff.entry(wallet_ctx).or_insert(0) -= pre_balance;

                        Ok(balances_diff)
                    },
                )
            })
            .unwrap_or_else(|| Ok(HashMap::default()))
    }
}
