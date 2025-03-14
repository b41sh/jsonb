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

use std::cmp::Ordering;

use crate::core::Deserializer;
use crate::error::*;
use crate::to_owned_jsonb;
use crate::Number;
use crate::OwnedJsonb;

use core::ops::Range;

/// Represents JSONB data wrapped around a raw, immutable slice of bytes.
///
/// It does not own the underlying data, allowing various operations to be performed on the JSONB data *without copying*.
/// This is critical for performance when dealing with large JSONB values.
/// `RawJsonb` provides various methods to inspect and manipulate the JSONB data efficiently.
//#[derive(Debug, Clone, Ord, PartialOrd, PartialEq, Eq)]
//#[derive(Debug, Clone, Copy, PartialEq)]
#[derive(Debug, Clone, Copy)]
pub struct RawJsonb<'a> {
    /// The underlying byte slice representing the JSONB data.
    pub(crate) data: &'a [u8],
}

impl<'a> RawJsonb<'a> {
    /// Creates a new RawJsonb from a byte slice.
    ///
    /// # Arguments
    ///
    /// * `data` - The byte slice containing the JSONB data.
    ///
    /// # Returns
    ///
    /// A new `RawJsonb` instance.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Checks if the JSONB data is empty.
    ///
    /// # Returns
    ///
    /// `true` if the data is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the length of the JSONB data in bytes.
    ///
    /// # Returns
    ///
    /// The length of the data in bytes.
    pub fn len(&self) -> usize {
        self.data.as_ref().len()
    }

    pub fn to_owned(&self) -> OwnedJsonb {
        OwnedJsonb::new(self.data.to_vec())
    }

    pub(crate) fn read_u32(&self, idx: usize) -> Result<u32> {
        let bytes: [u8; 4] = self
            .data
            .get(idx..idx + 4)
            .ok_or(Error::InvalidEOF)?
            .try_into()
            .unwrap();
        Ok(u32::from_be_bytes(bytes))
    }

    pub(crate) fn slice(&self, range: Range<usize>) -> Result<&'a [u8]> {
        // Check for potential out-of-bounds access before creating item
        if range.end > self.len() {
            return Err(Error::InvalidJsonb);
        }
        Ok(&self.data[range])
    }
}

/// Converts a borrowed byte slice into a RawJsonb.
/// This provides a convenient way to create a RawJsonb from existing data without copying.
impl<'a> From<&'a [u8]> for RawJsonb<'a> {
    fn from(data: &'a [u8]) -> Self {
        Self { data }
    }
}

/// Allows accessing the underlying byte slice as a reference.
/// This enables easy integration with functions that expect a &[u8].
impl AsRef<[u8]> for RawJsonb<'_> {
    fn as_ref(&self) -> &[u8] {
        self.data
    }
}

pub fn from_raw_jsonb<'de, T>(raw_jsonb: &'de RawJsonb) -> Result<T>
where
    T: serde::de::Deserialize<'de>,
{
    let mut deserializer = Deserializer::new(raw_jsonb);
    let t = T::deserialize(&mut deserializer)?;
    if deserializer.end() {
        Ok(t)
    } else {
        // Trailing characters
        Err(Error::InvalidJsonb)
    }
}

/// The value type of JSONB data.
#[derive(Debug, Clone, Copy)]
pub enum JsonbType {
    /// The Null JSON type.
    Null,
    /// The Boolean JSON type.
    Boolean,
    /// The Number JSON type.
    Number,
    /// The String JSON type.
    String,
    /// The Array JSON type with the length of items.
    Array(usize),
    /// The Object JSON type with the length of key and value pairs.
    Object(usize),
}

impl Eq for JsonbType {}

impl PartialEq for JsonbType {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for JsonbType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (JsonbType::Null, JsonbType::Null) => Some(Ordering::Equal),
            (JsonbType::Null, _) => Some(Ordering::Greater),
            (_, JsonbType::Null) => Some(Ordering::Less),

            (JsonbType::Array(_), JsonbType::Array(_)) => None,
            (JsonbType::Array(_), _) => Some(Ordering::Greater),
            (_, JsonbType::Array(_)) => Some(Ordering::Less),

            (JsonbType::Object(_), JsonbType::Object(_)) => None,
            (JsonbType::Object(_), _) => Some(Ordering::Greater),
            (_, JsonbType::Object(_)) => Some(Ordering::Less),

            (JsonbType::String, JsonbType::String) => None,
            (JsonbType::String, _) => Some(Ordering::Greater),
            (_, JsonbType::String) => Some(Ordering::Less),

            (JsonbType::Number, JsonbType::Number) => None,
            (JsonbType::Number, _) => Some(Ordering::Greater),
            (_, JsonbType::Number) => Some(Ordering::Less),

            (JsonbType::Boolean, JsonbType::Boolean) => None,
        }
    }
}

//#[derive(Debug, Clone, Copy, PartialEq)]
//#[derive(Debug, Clone, Ord, PartialOrd, PartialEq, Eq)]
#[derive(Debug, Clone)]
pub(crate) enum JsonbItem<'a> {
    Null,
    Boolean(bool),
    Number(&'a [u8]),
    String(&'a [u8]),
    Raw(RawJsonb<'a>),
    Owned(OwnedJsonb),
}

impl Eq for JsonbItem<'_> {}

impl PartialEq for JsonbItem<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for JsonbItem<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let self_type = self.jsonb_type().ok()?;
        let other_type = other.jsonb_type().ok()?;

        // First use JSONB type to determine the order,
        // different types must have different orders.
        if let Some(ord) = self_type.partial_cmp(&other_type) {
            return Some(ord);
        }

        let self_item = if let JsonbItem::Owned(owned) = self {
            &JsonbItem::Raw(owned.as_raw())
        } else {
            self
        };
        let other_item = if let JsonbItem::Owned(owned) = other {
            &JsonbItem::Raw(owned.as_raw())
        } else {
            other
        };

        match (self_item, other_item) {
            (JsonbItem::Raw(self_raw), JsonbItem::Raw(other_raw)) => {
                self_raw.partial_cmp(&other_raw)
            }
            // compare null, raw jsonb must not null
            (JsonbItem::Raw(_), JsonbItem::Null) => Some(Ordering::Less),
            (JsonbItem::Null, JsonbItem::Raw(_)) => Some(Ordering::Greater),
            // compare boolean
            (JsonbItem::Boolean(self_val), JsonbItem::Boolean(other_val)) => {
                self_val.partial_cmp(&other_val)
            }
            (JsonbItem::Raw(self_raw), JsonbItem::Boolean(other_val)) => {
                let self_val: Result<bool> = from_raw_jsonb(&self_raw);
                if let Ok(self_val) = self_val {
                    self_val.partial_cmp(&other_val)
                } else {
                    None
                }
            }
            (JsonbItem::Boolean(self_val), JsonbItem::Raw(other_raw)) => {
                let other_val: Result<bool> = from_raw_jsonb(&other_raw);
                if let Ok(other_val) = other_val {
                    self_val.partial_cmp(&other_val)
                } else {
                    None
                }
            }
            // compare number
            (JsonbItem::Number(self_data), JsonbItem::Number(other_data)) => {
                let self_num = Number::decode(self_data).ok()?;
                let other_num = Number::decode(other_data).ok()?;
                self_num.partial_cmp(&other_num)
            }
            (JsonbItem::Raw(self_raw), JsonbItem::Number(other_data)) => {
                let self_num: Result<Number> = from_raw_jsonb(&self_raw);
                let other_num = Number::decode(other_data).ok()?;
                if let Ok(self_num) = self_num {
                    self_num.partial_cmp(&other_num)
                } else {
                    None
                }
            }
            (JsonbItem::Number(self_data), JsonbItem::Raw(other_raw)) => {
                let self_num = Number::decode(self_data).ok()?;
                let other_num: Result<Number> = from_raw_jsonb(&other_raw);
                if let Ok(other_num) = other_num {
                    self_num.partial_cmp(&other_num)
                } else {
                    None
                }
            }
            // compare string
            (JsonbItem::String(self_data), JsonbItem::String(other_data)) => {
                let self_str = unsafe { std::str::from_utf8_unchecked(self_data) };
                let other_str = unsafe { std::str::from_utf8_unchecked(other_data) };
                self_str.partial_cmp(&other_str)
            }
            (JsonbItem::Raw(self_raw), JsonbItem::String(other_data)) => {
                let self_str: Result<String> = from_raw_jsonb(&self_raw);
                let other_str = unsafe { String::from_utf8_unchecked(other_data.to_vec()) };
                if let Ok(self_str) = self_str {
                    self_str.partial_cmp(&other_str)
                } else {
                    None
                }
            }
            (JsonbItem::String(self_data), JsonbItem::Raw(other_raw)) => {
                let self_str = unsafe { String::from_utf8_unchecked(self_data.to_vec()) };
                let other_str: Result<String> = from_raw_jsonb(&other_raw);
                if let Ok(other_str) = other_str {
                    self_str.partial_cmp(&other_str)
                } else {
                    None
                }
            }
            (_, _) => None,
        }
    }
}

impl Ord for JsonbItem<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.partial_cmp(other) {
            Some(ordering) => ordering,
            None => Ordering::Equal,
        }
    }
}

impl<'a> JsonbItem<'a> {
    pub(crate) fn as_raw_jsonb(&self) -> Option<RawJsonb<'a>> {
        match self {
            JsonbItem::Raw(raw_jsonb) => Some(*raw_jsonb),
            _ => None,
        }
    }

    pub(crate) fn to_owned_jsonb(&self) -> Result<OwnedJsonb> {
        let owned = match self {
            JsonbItem::Null => to_owned_jsonb(&())?,
            JsonbItem::Boolean(v) => to_owned_jsonb(&v)?,
            JsonbItem::Number(data) => {
                let n = Number::decode(data)?;
                match n {
                    Number::UInt64(v) => to_owned_jsonb(&v)?,
                    Number::Int64(v) => to_owned_jsonb(&v)?,
                    Number::Float64(v) => to_owned_jsonb(&v)?,
                }
            }
            JsonbItem::String(data) => {
                let s = unsafe { std::str::from_utf8_unchecked(data) };
                to_owned_jsonb(&s)?
            }
            JsonbItem::Raw(raw) => raw.to_owned(),
            JsonbItem::Owned(owned) => owned.clone(),
        };
        Ok(owned)
    }

    pub(crate) fn as_null(&self) -> Option<()> {
        match self {
            JsonbItem::Null => Some(()),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&'a str> {
        match self {
            JsonbItem::String(data) => {
                let s = unsafe { std::str::from_utf8_unchecked(data) };
                Some(s)
            }
            _ => None,
        }
    }
}
