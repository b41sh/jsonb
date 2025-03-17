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

use crate::RawJsonb;
use crate::OwnedJsonb;
use crate::to_owned_jsonb;

use core::ops::Range;
use std::io::Write;
use byteorder::BigEndian;
use byteorder::WriteBytesExt;
use super::constants::*;
use super::jentry::JEntry;
use crate::error::*;
use crate::Number;

/// The value type of JSONB data.
#[derive(Debug, Clone, Copy)]
pub(crate) enum JsonbType {
    /// The Null JSONB type.
    Null,
    /// The Boolean JSONB type.
    Boolean,
    /// The Number JSONB type.
    Number,
    /// The String JSONB type.
    String,
    /// The Array JSONB type with the length of items.
    Array(usize),
    /// The Object JSONB type with the length of key and value pairs.
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
    pub(crate) fn from_raw_jsonb(raw_jsonb: RawJsonb<'a>) -> Result<JsonbItem<'a>> {
        let (header_type, _) = raw_jsonb.read_header(0)?;
        match header_type {
            SCALAR_CONTAINER_TAG => {
                let jentry = raw_jsonb.read_jentry(4)?;
                let range = Range {
                    start: 8,
                    end: raw_jsonb.len(),
                };
                let data = raw_jsonb.slice(range)?;
                let item = match jentry.type_code {
                    NULL_TAG => JsonbItem::Null,
                    TRUE_TAG => JsonbItem::Boolean(true),
                    FALSE_TAG => JsonbItem::Boolean(false),
                    NUMBER_TAG => JsonbItem::Number(data),
                    STRING_TAG => JsonbItem::String(data),
                    _ => {
                        return Err(Error::InvalidJsonb);
                    }
                };
                Ok(item)
            }
            OBJECT_CONTAINER_TAG | ARRAY_CONTAINER_TAG => Ok(JsonbItem::Raw(raw_jsonb)),
            _ => Err(Error::InvalidJsonb),
        }
    }

    pub(crate) fn jsonb_type(&self) -> Result<JsonbType> {
        match self {
            JsonbItem::Null => Ok(JsonbType::Null),
            JsonbItem::Boolean(_) => Ok(JsonbType::Boolean),
            JsonbItem::Number(_) => Ok(JsonbType::Number),
            JsonbItem::String(_) => Ok(JsonbType::String),
            JsonbItem::Raw(raw) => raw.jsonb_type(),
            JsonbItem::Owned(owned) => owned.as_raw().jsonb_type(),
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

    pub(crate) fn as_raw_jsonb(&self) -> Option<RawJsonb<'a>> {
        match self {
            JsonbItem::Raw(raw_jsonb) => Some(*raw_jsonb),
            _ => None,
        }
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

impl RawJsonb<'_> {
    pub(crate) fn jsonb_type(&self) -> Result<JsonbType> {
        let mut index = 0;
        let (header_type, header_len) = self.read_header(index)?;
        index += 4;
        match header_type {
            SCALAR_CONTAINER_TAG => {
                let jentry = self.read_jentry(index)?;

                match jentry.type_code {
                    NULL_TAG => Ok(JsonbType::Null),
                    TRUE_TAG => Ok(JsonbType::Boolean),
                    FALSE_TAG => Ok(JsonbType::Boolean),
                    NUMBER_TAG => Ok(JsonbType::Number),
                    STRING_TAG => Ok(JsonbType::String),
                    _ => Err(Error::InvalidJsonb),
                }
            }
            ARRAY_CONTAINER_TAG => Ok(JsonbType::Array(header_len as usize)),
            OBJECT_CONTAINER_TAG => Ok(JsonbType::Object(header_len as usize)),
            _ => Err(Error::InvalidJsonb),
        }
    }

    pub(crate) fn read_header(&self, index: usize) -> Result<(u32, u32)> {
        let header = self.read_u32(index)?;
        let header_type = header & CONTAINER_HEADER_TYPE_MASK;
        match header_type {
            SCALAR_CONTAINER_TAG | OBJECT_CONTAINER_TAG | ARRAY_CONTAINER_TAG => {}
            _ => {
                return Err(Error::InvalidJsonb);
            }
        }
        let header_len = header & CONTAINER_HEADER_LEN_MASK;
        Ok((header_type, header_len))
    }

    pub(super) fn read_jentry(&self, index: usize) -> Result<JEntry> {
        let jentry_encoded = self.read_u32(index)?;
        let jentry = JEntry::decode_jentry(jentry_encoded);
        Ok(jentry)
    }
}

impl OwnedJsonb {
    pub(crate) fn from_item(item: JsonbItem<'_>) -> Result<OwnedJsonb> {
        let (jentry, data) = match item {
            JsonbItem::Null => {
                let jentry = JEntry::make_null_jentry();
                (jentry, None)
            }
            JsonbItem::Boolean(v) => {
                let jentry = if v {
                    JEntry::make_true_jentry()
                } else {
                    JEntry::make_false_jentry()
                };
                (jentry, None)
            }
            JsonbItem::Number(data) => {
                let jentry = JEntry::make_number_jentry(data.len());
                (jentry, Some(data))
            }
            JsonbItem::String(data) => {
                let jentry = JEntry::make_string_jentry(data.len());
                (jentry, Some(data))
            }
            JsonbItem::Raw(raw_jsonb) => {
                return Ok(raw_jsonb.to_owned());
            }
            JsonbItem::Owned(owned_jsonb) => {
                return Ok(owned_jsonb.clone());
            }
        };

        let len = if let Some(data) = data {
            data.len() + 8
        } else {
            8
        };
        let mut buf = Vec::with_capacity(len);
        let header = SCALAR_CONTAINER_TAG;
        buf.write_u32::<BigEndian>(header)?;
        buf.write_u32::<BigEndian>(jentry.encoded())?;
        if let Some(data) = data {
            buf.extend_from_slice(data);
        }
        Ok(OwnedJsonb::new(buf))
    }
}

impl Number {
    #[inline]
    pub(crate) fn compact_encode<W: Write>(&self, mut writer: W) -> Result<usize> {
        match self {
            Self::Int64(v) => {
                if *v == 0 {
                    writer.write_all(&[NUMBER_ZERO])?;
                    return Ok(1);
                }
                writer.write_all(&[NUMBER_INT])?;
                if *v >= i8::MIN.into() && *v <= i8::MAX.into() {
                    writer.write_all(&(*v as i8).to_be_bytes())?;
                    Ok(2)
                } else if *v >= i16::MIN.into() && *v <= i16::MAX.into() {
                    writer.write_all(&(*v as i16).to_be_bytes())?;
                    Ok(3)
                } else if *v >= i32::MIN.into() && *v <= i32::MAX.into() {
                    writer.write_all(&(*v as i32).to_be_bytes())?;
                    Ok(5)
                } else {
                    writer.write_all(&v.to_be_bytes())?;
                    Ok(9)
                }
            }
            Self::UInt64(v) => {
                if *v == 0 {
                    writer.write_all(&[NUMBER_ZERO])?;
                    return Ok(1);
                }
                writer.write_all(&[NUMBER_UINT])?;
                if *v <= u8::MAX.into() {
                    writer.write_all(&(*v as u8).to_be_bytes())?;
                    Ok(2)
                } else if *v <= u16::MAX.into() {
                    writer.write_all(&(*v as u16).to_be_bytes())?;
                    Ok(3)
                } else if *v <= u32::MAX.into() {
                    writer.write_all(&(*v as u32).to_be_bytes())?;
                    Ok(5)
                } else {
                    writer.write_all(&v.to_be_bytes())?;
                    Ok(9)
                }
            }
            Self::Float64(v) => {
                if v.is_nan() {
                    writer.write_all(&[NUMBER_NAN])?;
                    return Ok(1);
                } else if v.is_infinite() {
                    if v.is_sign_negative() {
                        writer.write_all(&[NUMBER_NEG_INF])?;
                    } else {
                        writer.write_all(&[NUMBER_INF])?;
                    }
                    return Ok(1);
                }
                writer.write_all(&[NUMBER_FLOAT])?;
                writer.write_all(&v.to_be_bytes())?;
                Ok(9)
            }
        }
    }

    #[inline]
    pub(crate) fn decode(bytes: &[u8]) -> Result<Number> {
        let mut len = bytes.len();
        assert!(len > 0);
        len -= 1;

        let ty = bytes[0];
        let num = match ty {
            NUMBER_ZERO => Number::UInt64(0),
            NUMBER_NAN => Number::Float64(f64::NAN),
            NUMBER_INF => Number::Float64(f64::INFINITY),
            NUMBER_NEG_INF => Number::Float64(f64::NEG_INFINITY),
            NUMBER_INT => match len {
                1 => Number::Int64(i8::from_be_bytes(bytes[1..].try_into().unwrap()) as i64),
                2 => Number::Int64(i16::from_be_bytes(bytes[1..].try_into().unwrap()) as i64),
                4 => Number::Int64(i32::from_be_bytes(bytes[1..].try_into().unwrap()) as i64),
                8 => Number::Int64(i64::from_be_bytes(bytes[1..].try_into().unwrap())),
                _ => {
                    return Err(Error::InvalidJsonbNumber);
                }
            },
            NUMBER_UINT => match len {
                1 => Number::UInt64(u8::from_be_bytes(bytes[1..].try_into().unwrap()) as u64),
                2 => Number::UInt64(u16::from_be_bytes(bytes[1..].try_into().unwrap()) as u64),
                4 => Number::UInt64(u32::from_be_bytes(bytes[1..].try_into().unwrap()) as u64),
                8 => Number::UInt64(u64::from_be_bytes(bytes[1..].try_into().unwrap())),
                _ => {
                    return Err(Error::InvalidJsonbNumber);
                }
            },
            NUMBER_FLOAT => Number::Float64(f64::from_be_bytes(bytes[1..].try_into().unwrap())),
            _ => {
                return Err(Error::InvalidJsonbNumber);
            }
        };
        Ok(num)
    }
}


