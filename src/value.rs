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

use crate::constants::*;
use crate::Error;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::mem::discriminant;

use super::number::Number;
use super::ser::Encoder;

pub type Object<'a> = BTreeMap<String, Value<'a>>;

// JSONB value
#[derive(Clone, PartialEq, Default, Eq)]
pub enum Value<'a> {
    #[default]
    Null,
    Bool(bool),
    String(Cow<'a, str>),
    Number(Number),
    Array(Vec<Value<'a>>),
    Object(Object<'a>),
}

impl<'a> Debug for Value<'a> {
    fn fmt(&self, formatter: &mut Formatter) -> std::fmt::Result {
        match *self {
            Value::Null => formatter.debug_tuple("Null").finish(),
            Value::Bool(v) => formatter.debug_tuple("Bool").field(&v).finish(),
            Value::Number(ref v) => Debug::fmt(v, formatter),
            Value::String(ref v) => formatter.debug_tuple("String").field(v).finish(),
            Value::Array(ref v) => {
                formatter.write_str("Array(")?;
                Debug::fmt(v, formatter)?;
                formatter.write_str(")")
            }
            Value::Object(ref v) => {
                formatter.write_str("Object(")?;
                Debug::fmt(v, formatter)?;
                formatter.write_str(")")
            }
        }
    }
}

impl<'a> Display for Value<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(v) => {
                if *v {
                    write!(f, "true")
                } else {
                    write!(f, "false")
                }
            }
            Value::Number(ref v) => write!(f, "{}", v),
            Value::String(ref v) => {
                write!(f, "{:?}", v)
            }
            Value::Array(ref vs) => {
                let mut first = true;
                write!(f, "[")?;
                for v in vs.iter() {
                    if !first {
                        write!(f, ",")?;
                    }
                    first = false;
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Object(ref vs) => {
                let mut first = true;
                write!(f, "{{")?;
                for (k, v) in vs.iter() {
                    if !first {
                        write!(f, ",")?;
                    }
                    first = false;
                    write!(f, "\"")?;
                    write!(f, "{k}")?;
                    write!(f, "\"")?;
                    write!(f, ":")?;
                    write!(f, "{v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

impl<'a> Value<'a> {
    pub fn is_scalar(&self) -> bool {
        !self.is_array() && !self.is_object()
    }

    pub fn is_object(&self) -> bool {
        self.as_object().is_some()
    }

    pub fn as_object(&self) -> Option<&Object<'a>> {
        match self {
            Value::Object(ref obj) => Some(obj),
            _ => None,
        }
    }

    pub fn is_array(&self) -> bool {
        self.as_array().is_some()
    }

    pub fn as_array(&self) -> Option<&Vec<Value<'a>>> {
        match self {
            Value::Array(ref array) => Some(array),
            _ => None,
        }
    }

    pub fn is_string(&self) -> bool {
        self.as_str().is_some()
    }

    pub fn as_str(&self) -> Option<&Cow<'_, str>> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    pub fn as_number(&self) -> Option<&Number> {
        match self {
            Value::Number(n) => Some(n),
            _ => None,
        }
    }

    pub fn is_i64(&self) -> bool {
        self.as_i64().is_some()
    }

    pub fn is_u64(&self) -> bool {
        self.as_u64().is_some()
    }

    pub fn is_f64(&self) -> bool {
        self.as_f64().is_some()
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(n) => n.as_i64(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) => n.as_u64(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => n.as_f64(),
            _ => None,
        }
    }

    pub fn is_boolean(&self) -> bool {
        self.as_bool().is_some()
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        self.as_null().is_some()
    }

    pub fn as_null(&self) -> Option<()> {
        match self {
            Value::Null => Some(()),
            _ => None,
        }
    }

    /// Cast `JSONB` value to Boolean
    pub fn to_bool(&self) -> Result<bool, Error> {
        if let Some(v) = self.as_bool() {
            return Ok(v);
        } else if let Some(v) = self.as_str() {
            if &v.to_lowercase() == "true" {
                return Ok(true);
            } else if &v.to_lowercase() == "false" {
                return Ok(false);
            }
        }
        Err(Error::InvalidCast)
    }

    /// Cast `JSONB` value to f64
    pub fn to_f64(&self) -> Result<f64, Error> {
        if let Some(v) = self.as_f64() {
            return Ok(v);
        } else if let Some(v) = self.as_bool() {
            if v {
                return Ok(1_f64);
            } else {
                return Ok(0_f64);
            }
        } else if let Some(v) = self.as_str() {
            if let Ok(v) = v.parse::<f64>() {
                return Ok(v);
            }
        }
        Err(Error::InvalidCast)
    }

    /// Cast `JSONB` value to i64
    pub fn to_i64(&self) -> Result<i64, Error> {
        if let Some(v) = self.as_i64() {
            return Ok(v);
        } else if let Some(v) = self.as_bool() {
            if v {
                return Ok(1_i64);
            } else {
                return Ok(0_i64);
            }
        } else if let Some(v) = self.as_str() {
            if let Ok(v) = v.parse::<i64>() {
                return Ok(v);
            }
        }
        Err(Error::InvalidCast)
    }

    /// Cast `JSONB` value to u64
    pub fn to_u64(&self) -> Result<u64, Error> {
        if let Some(v) = self.as_u64() {
            return Ok(v);
        } else if let Some(v) = self.as_bool() {
            if v {
                return Ok(1_u64);
            } else {
                return Ok(0_u64);
            }
        } else if let Some(v) = self.as_str() {
            if let Ok(v) = v.parse::<u64>() {
                return Ok(v);
            }
        }
        Err(Error::InvalidCast)
    }

    /// Cast `JSONB` value to String
    pub fn to_str(&self) -> Result<String, Error> {
        if let Some(v) = self.as_str() {
            return Ok(v.to_string());
        } else if let Some(v) = self.as_bool() {
            if v {
                return Ok("true".to_string());
            } else {
                return Ok("false".to_string());
            }
        } else if let Some(v) = self.as_number() {
            return Ok(format!("{}", v));
        }
        Err(Error::InvalidCast)
    }

    /// Returns the type of the top-level JSON value as a text string.
    /// Possible types are object, array, string, number, boolean, and null.
    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Null => TYPE_NULL,
            Value::Bool(_) => TYPE_BOOLEAN,
            Value::String(_) => TYPE_STRING,
            Value::Number(_) => TYPE_NUMBER,
            Value::Array(_) => TYPE_ARRAY,
            Value::Object(_) => TYPE_OBJECT,
        }
    }

    /// Serialize the JSONB Value into a byte stream.
    pub fn write_to_vec(&self, buf: &mut Vec<u8>) {
        let mut encoder = Encoder::new(buf);
        encoder.encode(self);
    }

    /// Serialize the JSONB Value into a byte stream.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_to_vec(&mut buf);
        buf
    }

    /// if `ignore_case` is true, enables case-insensitive matching.
    pub fn get_by_name(&self, name: &str, ignore_case: bool) -> Option<&Value<'a>> {
        match self {
            Value::Object(obj) => match obj.get(name) {
                Some(val) => Some(val),
                None => {
                    if ignore_case {
                        for key in obj.keys() {
                            if name.eq_ignore_ascii_case(key) {
                                return obj.get(key);
                            }
                        }
                    }
                    None
                }
            },
            _ => None,
        }
    }

    /// Get the inner element of `JSONB` Array by index.
    pub fn get_by_index(&self, index: usize) -> Option<&Value<'a>> {
        match self {
            Value::Array(arr) => {
                if index >= arr.len() {
                    return None;
                }
                Some(&arr[index])
            },
            _ => None,
        }
    }

    pub fn array_length(&self) -> Option<usize> {
        match self {
            Value::Array(arr) => Some(arr.len()),
            _ => None,
        }
    }

    pub fn object_keys(&self) -> Option<Value<'a>> {
        match self {
            Value::Object(obj) => {
                let mut keys = Vec::with_capacity(obj.len());
                for k in obj.keys() {
                    keys.push(k.clone().into());
                }
                Some(Value::Array(keys))
            }
            _ => None,
        }
    }

    pub fn eq_variant(&self, other: &Value) -> bool {
        discriminant(self) == discriminant(other)
    }
}
