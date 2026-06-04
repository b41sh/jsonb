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

use jaq_core::data::DataT;

use crate::core::QueryValue;

/// jaq data marker for JSONB-backed values.
pub struct JsonbData;

impl DataT for JsonbData {
    type V<'a> = QueryValue<'a>;
    type Data<'a> = &'a jaq_core::Lut<Self>;
}
