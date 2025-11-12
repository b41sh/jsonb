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

use core::iter::FromIterator;
use std::borrow::Cow;

use ethnum::i256;
use ordered_float::OrderedFloat;
use serde_json::Map as JsonMap;
use serde_json::Number as JsonNumber;
use serde_json::Value as JsonValue;

use crate::value::Object;
use crate::value::Value;
use crate::Decimal128;
use crate::Decimal256;
use crate::Number;

macro_rules! from_signed_integer {
    ($($ty:ident)*) => {
        $(
            impl<'a> From<$ty> for Value<'a> {
                fn from(n: $ty) -> Self {
                    Value::Number(Number::Int64(n as i64))
                }
            }
        )*
    };
}

macro_rules! from_unsigned_integer {
    ($($ty:ident)*) => {
        $(
            impl<'a> From<$ty> for Value<'a> {
                fn from(n: $ty) -> Self {
                    Value::Number(Number::UInt64(n as u64))
                }
            }
        )*
    };
}

macro_rules! from_float {
    ($($ty:ident)*) => {
        $(
            impl<'a> From<$ty> for Value<'a> {
                fn from(n: $ty) -> Self {
                    Value::Number(Number::Float64(n as f64))
                }
            }
        )*
    };
}

from_signed_integer! {
    i8 i16 i32 i64 isize
}

from_unsigned_integer! {
    u8 u16 u32 u64 usize
}

from_float! {
    f32 f64
}

impl From<OrderedFloat<f32>> for Value<'_> {
    fn from(f: OrderedFloat<f32>) -> Self {
        Value::Number(Number::Float64(f.0 as f64))
    }
}

impl From<OrderedFloat<f64>> for Value<'_> {
    fn from(f: OrderedFloat<f64>) -> Self {
        Value::Number(Number::Float64(f.0))
    }
}

impl From<bool> for Value<'_> {
    fn from(f: bool) -> Self {
        Value::Bool(f)
    }
}

impl From<String> for Value<'_> {
    fn from(f: String) -> Self {
        Value::String(f.into())
    }
}

impl<'a> From<&'a str> for Value<'a> {
    fn from(f: &'a str) -> Self {
        Value::String(Cow::from(f))
    }
}

impl<'a> From<Cow<'a, str>> for Value<'a> {
    fn from(f: Cow<'a, str>) -> Self {
        Value::String(f)
    }
}

impl<'a> From<Object<'a>> for Value<'a> {
    fn from(o: Object<'a>) -> Self {
        Value::Object(o)
    }
}

impl<'a, T: Into<Value<'a>>> From<Vec<T>> for Value<'a> {
    fn from(f: Vec<T>) -> Self {
        Value::Array(f.into_iter().map(Into::into).collect())
    }
}

impl<'a, T: Clone + Into<Value<'a>>> From<&'a [T]> for Value<'a> {
    fn from(f: &'a [T]) -> Self {
        Value::Array(f.iter().cloned().map(Into::into).collect())
    }
}

impl<'a, T: Into<Value<'a>>> FromIterator<T> for Value<'a> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Value::Array(iter.into_iter().map(Into::into).collect())
    }
}

impl<'a, K: Into<String>, V: Into<Value<'a>>> FromIterator<(K, V)> for Value<'a> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Value::Object(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl From<()> for Value<'_> {
    fn from((): ()) -> Self {
        Value::Null
    }
}

impl From<&JsonValue> for Value<'_> {
    fn from(value: &JsonValue) -> Self {
        match value {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(v) => Value::Bool(*v),
            JsonValue::Number(v) => {
                if let Some(n) = v.as_u64() {
                    Value::Number(Number::UInt64(n))
                } else if let Some(n) = v.as_i64() {
                    Value::Number(Number::Int64(n))
                } else if let Some(n) = v.as_i128() {
                    Value::Number(Number::Decimal128(Decimal128 { value: n, scale: 0 }))
                } else if let Some(n) = v.as_u128() {
                    Value::Number(Number::Decimal256(Decimal256 {
                        value: n.into(),
                        scale: 0,
                    }))
                } else if let Some(n) = v.as_f64() {
                    Value::Number(Number::Float64(n))
                } else {
                    // If the value is NaN or Infinity, fallback to NULL
                    Value::Null
                }
            }
            JsonValue::String(v) => Value::String(v.clone().into()),
            JsonValue::Array(arr) => {
                let mut vals: Vec<Value> = Vec::with_capacity(arr.len());
                for val in arr {
                    vals.push(val.into());
                }
                Value::Array(vals)
            }
            JsonValue::Object(obj) => {
                let mut map = Object::new();
                for (k, v) in obj.iter() {
                    let val: Value = v.into();
                    map.insert(k.to_string(), val);
                }
                Value::Object(map)
            }
        }
    }
}

impl From<JsonValue> for Value<'_> {
    fn from(value: JsonValue) -> Self {
        (&value).into()
    }
}

impl<'a> From<Value<'a>> for JsonValue {
    fn from(value: Value<'a>) -> Self {
        match value {
            Value::Null => JsonValue::Null,
            Value::Bool(v) => JsonValue::Bool(v),
            Value::Number(v) => match v {
                Number::Int64(n) => JsonValue::Number(n.into()),
                Number::UInt64(n) => JsonValue::Number(n.into()),
                Number::Decimal64(d) if d.scale == 0 => JsonValue::Number(d.value.into()),
                #[cfg(feature = "arbitrary_precision")]
                Number::Decimal128(ref d) if d.scale == 0 => {
                    if let Some(n) = JsonNumber::from_i128(d.value) {
                        JsonValue::Number(n)
                    } else if let Some(n) = JsonNumber::from_f64(v.as_f64()) {
                        JsonValue::Number(n)
                    } else {
                        JsonValue::Null
                    }
                }
                #[cfg(feature = "arbitrary_precision")]
                Number::Decimal256(ref d) if d.scale == 0 => {
                    if d.value >= i256::ZERO && d.value <= i256::from(u128::MAX) {
                        if let Some(n) = JsonNumber::from_u128(d.value.as_u128()) {
                            return JsonValue::Number(n);
                        }
                    } else if d.value >= i256::from(i128::MIN) && d.value < i256::ZERO {
                        if let Some(n) = JsonNumber::from_i128(d.value.as_i128()) {
                            return JsonValue::Number(n);
                        }
                    } else if let Some(n) = JsonNumber::from_f64(v.as_f64()) {
                        return JsonValue::Number(n);
                    }
                    JsonValue::Null
                }
                _ => {
                    if let Some(n) = JsonNumber::from_f64(v.as_f64()) {
                        JsonValue::Number(n)
                    } else {
                        // If the value is NaN or Infinity, fallback to NULL
                        JsonValue::Null
                    }
                }
            },
            Value::String(v) => JsonValue::String(v.to_string()),
            Value::Binary(v) => {
                let mut s = String::new();
                for c in v {
                    s.push_str(&format!("{c:02X}"));
                }
                JsonValue::String(s)
            }
            Value::Date(v) => {
                let s = format!("{}", v);
                JsonValue::String(s)
            }
            Value::Timestamp(v) => {
                let s = format!("{}", v);
                JsonValue::String(s)
            }
            Value::TimestampTz(v) => {
                let s = format!("{}", v);
                JsonValue::String(s)
            }
            Value::Interval(v) => {
                let s = format!("{}", v);
                JsonValue::String(s)
            }
            Value::Array(arr) => {
                let mut vals: Vec<JsonValue> = Vec::with_capacity(arr.len());
                for val in arr {
                    vals.push(val.into());
                }
                JsonValue::Array(vals)
            }
            Value::Object(obj) => {
                let mut map = JsonMap::new();
                for (k, v) in obj.iter() {
                    let val: JsonValue = v.clone().into();
                    map.insert(k.to_string(), val);
                }
                JsonValue::Object(map)
            }
        }
    }
}
