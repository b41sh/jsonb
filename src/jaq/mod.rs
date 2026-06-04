// Copyright 2023 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! jaq integration for executing filters directly over JSONB values.

mod access;
mod data;
mod funs;
mod raw;
mod value;

#[cfg(test)]
mod compat_tests;

pub use crate::core::QueryValue;
pub use data::JsonbData;
pub use funs::defs;
pub use funs::funs;
