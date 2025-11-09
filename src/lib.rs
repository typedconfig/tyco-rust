//! Tyco configuration language parser – Rust implementation.
//! 
//! This crate mirrors the behaviour of the reference Python parser and is kept
//! in sync with the shared test suite that lives in `../tyco-test-suite`.

mod context;
mod error;
mod parser;
mod utils;
mod value;

pub use context::{FieldSchema, TycoContext, TycoStruct};
pub use error::TycoError;
pub use parser::{load, loads, TycoParser};
pub use value::{TycoInstance, TycoString, TycoValue};
