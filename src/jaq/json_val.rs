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

use std::borrow::Cow;
use std::str;

use jaq_core::load;
use jaq_core::native::Fun;
use jaq_json::Map;
use jaq_json::Num;
use jaq_json::Val;
use num_traits::ToPrimitive;

use crate::core::ArrayBuilder;
use crate::core::ArrayIterator;
use crate::core::ExtensionItem;
use crate::core::JsonbItem;
use crate::core::NumberItem;
use crate::core::ObjectBuilder;
use crate::core::ObjectIterator;
use crate::error::Result;
use crate::Error;
use crate::ExtensionValue;
use crate::Number;
use crate::OwnedJsonb;
use crate::RawJsonb;

/// jaq-json value type used for materialized jaq execution.
pub type JsonVal = Val;

/// jaq data marker for jaq-json values.
pub type JsonValData = jaq_core::data::JustLut<JsonVal>;

/// Complete jaq definitions for jaq-json value execution.
pub fn json_val_defs() -> impl Iterator<Item = load::parse::Def<&'static str>> {
    jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs())
}

/// Complete jaq native functions for jaq-json value execution.
pub fn json_val_funs() -> impl Iterator<Item = Fun<JsonValData>> {
    jaq_core::funs::<JsonValData>()
        .chain(jaq_std::funs::<JsonValData>())
        .chain(jaq_json::funs::<JsonValData>())
}

/// Convert raw JSONB directly into jaq-json's native value type.
pub fn raw_jsonb_to_jaq_val(raw: RawJsonb<'_>) -> Result<Val> {
    jsonb_item_to_jaq_val(JsonbItem::from_raw_jsonb(raw)?)
}

/// Convert a jaq-json value back into JSONB.
pub fn jaq_val_to_owned_jsonb(value: &Val) -> Result<OwnedJsonb> {
    match value {
        Val::Arr(values) => {
            let mut builder = ArrayBuilder::with_capacity(values.len());
            for value in values.iter() {
                builder.push_jsonb_item(jaq_val_to_jsonb_item(value)?);
            }
            builder.build()
        }
        Val::Obj(values) => {
            let mut entries = Vec::with_capacity(values.len());
            for (key, value) in values.iter() {
                let key = jaq_val_to_object_key(key)?;
                entries.push((key.into_owned(), jaq_val_to_jsonb_item(value)?));
            }
            ObjectBuilder::build_from_entries(entries)
        }
        value => OwnedJsonb::from_item(jaq_val_to_jsonb_item(value)?),
    }
}

fn jsonb_item_to_jaq_val(item: JsonbItem<'_>) -> Result<Val> {
    match item {
        JsonbItem::Null => Ok(Val::Null),
        JsonbItem::Boolean(value) => Ok(Val::Bool(value)),
        JsonbItem::Number(value) => value.as_number().map(number_to_jaq_num).map(Val::Num),
        JsonbItem::String(value) => Ok(Val::utf8_str(value.as_bytes().to_vec())),
        JsonbItem::Extension(value) => match value.as_extension_value()? {
            ExtensionValue::Binary(value) => Ok(Val::byte_str(value.to_vec())),
            value => Ok(Val::utf8_str(value.to_string().into_bytes())),
        },
        JsonbItem::Raw(raw) => raw_container_to_jaq_val(raw),
        JsonbItem::Owned(owned) => raw_jsonb_to_jaq_val(owned.as_raw()),
    }
}

fn raw_container_to_jaq_val(raw: RawJsonb<'_>) -> Result<Val> {
    if let Some(iter) = ArrayIterator::new(raw)? {
        let values = iter
            .map(|item| item.and_then(jsonb_item_to_jaq_val))
            .collect::<Result<Vec<_>>>()?;
        return Ok(values.into_iter().collect());
    }

    if let Some(iter) = ObjectIterator::new(raw)? {
        let mut values = Map::default();
        for item in iter {
            let (key, value) = item?;
            values.insert(
                Val::utf8_str(key.as_bytes().to_vec()),
                jsonb_item_to_jaq_val(value)?,
            );
        }
        return Ok(Val::obj(values));
    }

    jsonb_item_to_jaq_val(JsonbItem::from_raw_jsonb(raw)?)
}

fn number_to_jaq_num(number: Number) -> Num {
    match number {
        Number::Int64(value) => Num::from_integral(value),
        Number::UInt64(value) => Num::from_integral(value),
        Number::Float64(value) => Num::Float(value),
        Number::Decimal64(_) | Number::Decimal128(_) | Number::Decimal256(_) => {
            Num::Dec(jaq_json::Rc::new(number.to_string()))
        }
    }
}

fn jaq_val_to_jsonb_item(value: &Val) -> Result<JsonbItem<'_>> {
    match value {
        Val::Null => Ok(JsonbItem::Null),
        Val::Bool(value) => Ok(JsonbItem::Boolean(*value)),
        Val::Num(value) => Ok(JsonbItem::Number(NumberItem::Number(jaq_num_to_number(
            value,
        )?))),
        Val::TStr(value) => Ok(JsonbItem::String(bytes_to_string(value.as_ref())?)),
        Val::BStr(value) => Ok(JsonbItem::Extension(ExtensionItem::Extension(
            ExtensionValue::Binary(value.as_ref()),
        ))),
        Val::Arr(_) | Val::Obj(_) => Ok(JsonbItem::Owned(jaq_val_to_owned_jsonb(value)?)),
    }
}

fn jaq_val_to_object_key(value: &Val) -> Result<Cow<'_, str>> {
    match value {
        Val::TStr(value) | Val::BStr(value) => bytes_to_string(value.as_ref()),
        _ => Err(Error::InvalidObject),
    }
}

fn bytes_to_string(value: &[u8]) -> Result<Cow<'_, str>> {
    str::from_utf8(value)
        .map(Cow::Borrowed)
        .map_err(|_| Error::InvalidObject)
}

fn jaq_num_to_number(value: &Num) -> Result<Number> {
    match value {
        Num::Int(value) => Ok(Number::Int64(*value as i64)),
        Num::BigInt(value) => {
            if let Some(value) = value.to_i64() {
                return Ok(Number::Int64(value));
            }
            if let Some(value) = value.to_u64() {
                return Ok(Number::UInt64(value));
            }
            Err(Error::InvalidJsonbNumber)
        }
        Num::Float(value) => Ok(Number::Float64(*value)),
        Num::Dec(value) => crate::parse_owned_jsonb(value.as_bytes())
            .and_then(|value| value.as_raw().as_number()?.ok_or(Error::InvalidJsonbNumber)),
    }
}
