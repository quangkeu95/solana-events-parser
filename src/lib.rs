#![allow(unstable_name_collisions)]

#[cfg(feature = "anchor")]
pub mod event_parser;
#[cfg(feature = "solana")]
pub mod instruction_parser;
#[cfg(feature = "solana")]
pub mod transaction_parser;

pub mod log_parser;

#[cfg(feature = "solana")]
pub use crate::transaction_parser::{BindTransactionInstructionLogs, BindTransactionLogs};
#[cfg(feature = "anchor")]
pub use crate::{event_parser::ParseEvent, instruction_parser::ParseInstruction};

#[cfg(feature = "storage")]
pub mod storage;

#[cfg(feature = "event-reader")]
pub mod event_reader_service;

#[cfg(feature = "solana")]
pub use de_solana_client;
