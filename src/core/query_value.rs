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
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::fmt::Formatter;
use std::rc::Rc;

use crate::core::JsonbItemType;
use crate::error::Result;
use crate::Error;
use crate::Number;
use crate::OwnedJsonb;
use crate::RawJsonb;
use crate::Value;

/// JSONB-backed value used by jaq.
#[derive(Clone, Debug)]
pub enum QueryValue<'a> {
    /// Borrowed JSONB subtree.
    Raw(RawJsonb<'a>),
    /// Shared owned JSONB subtree.
    Owned(Rc<OwnedJsonb>),
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number.
    Number(Number),
    /// UTF-8 string.
    String(Cow<'a, str>),
    /// Materialized array.
    Array(Rc<Vec<QueryValue<'a>>>),
    /// Materialized object. jaq allows arbitrary keys; JSONB output validates string keys.
    Object(Rc<BTreeMap<QueryValue<'a>, QueryValue<'a>>>),
}

impl<'a> QueryValue<'a> {
    pub fn from_raw(raw: RawJsonb<'a>) -> Self {
        Self::Raw(raw)
    }

    pub fn from_owned(owned: OwnedJsonb) -> Self {
        Self::Owned(Rc::new(owned))
    }

    pub fn into_owned_jsonb(self) -> Result<OwnedJsonb> {
        match self {
            Self::Raw(raw) => Ok(raw.to_owned()),
            Self::Owned(owned) => match Rc::try_unwrap(owned) {
                Ok(owned) => Ok(owned),
                Err(owned) => Ok((*owned).clone()),
            },
            Self::Array(values) => {
                let values = values
                    .iter()
                    .cloned()
                    .map(QueryValue::into_owned_jsonb)
                    .collect::<Result<Vec<_>>>()?;
                OwnedJsonb::build_array(values.iter().map(|value| value.as_raw()))
            }
            Self::Object(values) => {
                let entries = values
                    .iter()
                    .map(|(key, value)| {
                        let Some(key) = key.as_object_key()? else {
                            return Err(Error::InvalidObject);
                        };
                        Ok((key, value.clone().into_owned_jsonb()?))
                    })
                    .collect::<Result<Vec<_>>>()?;
                OwnedJsonb::build_object(
                    entries
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_raw())),
                )
            }
            value => {
                let value = value.into_jsonb_value()?;
                let mut buf = Vec::new();
                value.write_to_vec(&mut buf);
                Ok(OwnedJsonb::new(buf))
            }
        }
    }

    pub(crate) fn as_object_key(&self) -> Result<Option<String>> {
        self.as_key_string()
    }

    pub(crate) fn as_number(&self) -> Result<Option<Number>> {
        match self {
            Self::Number(value) => Ok(Some(value.clone())),
            Self::Raw(raw) => raw.as_number(),
            Self::Owned(owned) => owned.as_raw().as_number(),
            _ => Ok(None),
        }
    }

    pub(crate) fn as_string(&self) -> Result<Option<Cow<'_, str>>> {
        match self {
            Self::String(value) => Ok(Some(Cow::Borrowed(value.as_ref()))),
            Self::Raw(raw) => raw.as_str(),
            Self::Owned(owned) => owned
                .as_raw()
                .as_str()
                .map(|value| value.map(|value| Cow::Owned(value.into_owned()))),
            _ => Ok(None),
        }
    }

    pub(crate) fn as_key_string(&self) -> Result<Option<String>> {
        self.as_string()
            .map(|value| value.map(|value| value.into_owned()))
    }

    pub(crate) fn as_isize(&self) -> Result<Option<isize>> {
        match self.as_number()? {
            Some(Number::Int64(number)) => Ok(isize::try_from(number).ok()),
            Some(Number::UInt64(number)) => Ok(isize::try_from(number).ok()),
            _ => Ok(None),
        }
    }

    pub(crate) fn as_bool(&self) -> Result<Option<bool>> {
        match self {
            Self::Bool(value) => Ok(Some(*value)),
            Self::Raw(raw) => raw.as_bool(),
            Self::Owned(owned) => owned.as_raw().as_bool(),
            _ => Ok(None),
        }
    }

    pub(crate) fn into_owned_static(self) -> QueryValue<'static> {
        match self {
            Self::Raw(raw) => QueryValue::from_owned(raw.to_owned()),
            Self::Owned(owned) => QueryValue::Owned(owned),
            Self::Null => QueryValue::Null,
            Self::Bool(value) => QueryValue::Bool(value),
            Self::Number(value) => QueryValue::Number(value),
            Self::String(value) => QueryValue::String(Cow::Owned(value.into_owned())),
            Self::Array(values) => QueryValue::Array(Rc::new(
                values
                    .iter()
                    .cloned()
                    .map(QueryValue::into_owned_static)
                    .collect(),
            )),
            Self::Object(values) => QueryValue::Object(Rc::new(
                values
                    .iter()
                    .map(|(key, value)| {
                        (
                            key.clone().into_owned_static(),
                            value.clone().into_owned_static(),
                        )
                    })
                    .collect(),
            )),
        }
    }

    pub(crate) fn from_static(value: QueryValue<'static>) -> Self {
        match value {
            QueryValue::Raw(raw) => Self::from_owned(raw.to_owned()),
            QueryValue::Owned(owned) => Self::Owned(owned),
            QueryValue::Null => Self::Null,
            QueryValue::Bool(value) => Self::Bool(value),
            QueryValue::Number(value) => Self::Number(value),
            QueryValue::String(value) => Self::String(Cow::Owned(value.into_owned())),
            QueryValue::Array(values) => Self::Array(Rc::new(
                values
                    .iter()
                    .cloned()
                    .map(QueryValue::from_static)
                    .collect(),
            )),
            QueryValue::Object(values) => Self::Object(Rc::new(
                values
                    .iter()
                    .map(|(key, value)| {
                        (
                            QueryValue::from_static(key.clone()),
                            QueryValue::from_static(value.clone()),
                        )
                    })
                    .collect(),
            )),
        }
    }

    fn into_jsonb_value(self) -> Result<Value<'static>> {
        match self {
            Self::Null => Ok(Value::Null),
            Self::Bool(value) => Ok(Value::Bool(value)),
            Self::Number(value) => Ok(Value::Number(value)),
            Self::String(value) => Ok(Value::String(Cow::Owned(value.into_owned()))),
            Self::Array(values) => values
                .iter()
                .cloned()
                .map(QueryValue::into_jsonb_value)
                .collect::<Result<Vec<_>>>()
                .map(Value::Array),
            Self::Object(values) => {
                let mut object = BTreeMap::new();
                for (key, value) in values.iter() {
                    let Some(key) = key.as_object_key()? else {
                        return Err(Error::InvalidObject);
                    };
                    object.insert(key, value.clone().into_jsonb_value()?);
                }
                Ok(Value::Object(object))
            }
            Self::Raw(_) | Self::Owned(_) => Err(Error::InvalidCast),
        }
    }

    fn json_type_rank(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Bool(_) => 1,
            Self::Number(_) => 2,
            Self::String(_) => 3,
            Self::Object(_) => 4,
            Self::Array(_) => 5,
            Self::Raw(raw) => raw
                .jsonb_item_type()
                .map(|typ| match typ {
                    JsonbItemType::Null => 0,
                    JsonbItemType::Boolean => 1,
                    JsonbItemType::Number => 2,
                    JsonbItemType::String => 3,
                    JsonbItemType::Object(_) => 4,
                    JsonbItemType::Array(_) => 5,
                    JsonbItemType::Extension => 6,
                })
                .unwrap_or(6),
            Self::Owned(owned) => owned
                .as_raw()
                .jsonb_item_type()
                .map(|typ| match typ {
                    JsonbItemType::Null => 0,
                    JsonbItemType::Boolean => 1,
                    JsonbItemType::Number => 2,
                    JsonbItemType::String => 3,
                    JsonbItemType::Object(_) => 4,
                    JsonbItemType::Array(_) => 5,
                    JsonbItemType::Extension => 6,
                })
                .unwrap_or(6),
        }
    }

    fn to_json_string(&self) -> String {
        fn number_string(number: &Number) -> String {
            match number {
                Number::Float64(value) if value.is_nan() => "NaN".to_string(),
                Number::Float64(value) if value.is_infinite() && value.is_sign_positive() => {
                    "Infinity".to_string()
                }
                Number::Float64(value) if value.is_infinite() && value.is_sign_negative() => {
                    "-Infinity".to_string()
                }
                number => number.to_string(),
            }
        }

        match self {
            Self::Raw(raw) => raw
                .as_number()
                .ok()
                .flatten()
                .map(|number| number_string(&number))
                .unwrap_or_else(|| raw.to_string()),
            Self::Owned(owned) => owned
                .as_raw()
                .as_number()
                .ok()
                .flatten()
                .map(|number| number_string(&number))
                .unwrap_or_else(|| owned.as_raw().to_string()),
            Self::Null => "null".to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Number(value) => number_string(value),
            Self::String(value) => serde_json::to_string(value.as_ref())
                .unwrap_or_else(|_| format!("{:?}", value.as_ref())),
            Self::Array(values) => {
                let values = values
                    .iter()
                    .map(QueryValue::to_json_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{values}]")
            }
            Self::Object(values) => {
                let values = values
                    .iter()
                    .map(|(key, value)| {
                        let key = key
                            .as_object_key()
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| key.to_json_string());
                        let key =
                            serde_json::to_string(&key).unwrap_or_else(|_| format!("{key:?}"));
                        format!("{key}:{}", value.to_json_string())
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{{{values}}}")
            }
        }
    }
}

impl Display for QueryValue<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_json_string())
    }
}

impl PartialEq for QueryValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for QueryValue<'_> {}

impl PartialOrd for QueryValue<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueryValue<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        let rank = self.json_type_rank().cmp(&other.json_type_rank());
        if rank != Ordering::Equal {
            return rank;
        }

        match (self, other) {
            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Bool(left), Self::Bool(right)) => left.cmp(right),
            (Self::Number(left), Self::Number(right)) => left.cmp(right),
            (Self::String(left), Self::String(right)) => left.cmp(right),
            (Self::Array(left), Self::Array(right)) => left.cmp(right),
            (Self::Object(left), Self::Object(right)) => left.cmp(right),
            (Self::Raw(left), Self::Raw(right)) => left.cmp(right),
            (Self::Owned(left), Self::Owned(right)) => left.as_raw().cmp(&right.as_raw()),
            _ => match self.json_type_rank() {
                0 => Ordering::Equal,
                1 => self
                    .as_bool()
                    .ok()
                    .flatten()
                    .cmp(&other.as_bool().ok().flatten()),
                2 => self
                    .as_number()
                    .ok()
                    .flatten()
                    .cmp(&other.as_number().ok().flatten()),
                3 => self
                    .as_string()
                    .ok()
                    .flatten()
                    .cmp(&other.as_string().ok().flatten()),
                _ => self.to_json_string().cmp(&other.to_json_string()),
            },
        }
    }
}

impl From<bool> for QueryValue<'_> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<isize> for QueryValue<'_> {
    fn from(value: isize) -> Self {
        Self::Number(Number::Int64(value as i64))
    }
}

impl From<usize> for QueryValue<'_> {
    fn from(value: usize) -> Self {
        Self::Number(Number::UInt64(value as u64))
    }
}

impl From<f64> for QueryValue<'_> {
    fn from(value: f64) -> Self {
        Self::Number(Number::Float64(value))
    }
}

impl From<String> for QueryValue<'_> {
    fn from(value: String) -> Self {
        Self::String(Cow::Owned(value))
    }
}

impl<'a> From<std::ops::Range<Option<QueryValue<'a>>>> for QueryValue<'a> {
    fn from(range: std::ops::Range<Option<QueryValue<'a>>>) -> Self {
        let mut object = BTreeMap::new();
        if let Some(start) = range.start {
            object.insert(Self::from("start".to_string()), start);
        }
        if let Some(end) = range.end {
            object.insert(Self::from("end".to_string()), end);
        }
        Self::Object(Rc::new(object))
    }
}

impl<'a> FromIterator<QueryValue<'a>> for QueryValue<'a> {
    fn from_iter<T: IntoIterator<Item = QueryValue<'a>>>(iter: T) -> Self {
        Self::Array(Rc::new(iter.into_iter().collect()))
    }
}
