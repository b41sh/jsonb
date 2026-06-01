//! jaq integration for executing filters directly over JSONB values.

mod data;
mod funs;
mod value;

#[cfg(test)]
mod compat_tests;

pub use crate::core::QueryValue;
pub use data::JsonbData;
pub use funs::{defs, funs};
