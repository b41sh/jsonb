use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

use crate::{Error, Number, OwnedJsonb, RawJsonb, Value};

/// JSONB-backed value used by jaq.
#[derive(Clone, Debug)]
pub enum QueryValue<'a> {
    /// Borrowed JSONB subtree.
    Raw(RawJsonb<'a>),
    /// Owned JSONB subtree.
    Owned(OwnedJsonb),
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
        Self::Owned(owned)
    }

    pub fn into_owned_jsonb(self) -> crate::error::Result<OwnedJsonb> {
        match self {
            Self::Raw(raw) => Ok(raw.to_owned()),
            Self::Owned(owned) => Ok(owned),
            value => {
                let value = value.into_jsonb_value()?;
                let mut buf = Vec::new();
                value.write_to_vec(&mut buf);
                Ok(OwnedJsonb::new(buf))
            }
        }
    }

    pub(crate) fn as_object_key(&self) -> crate::error::Result<Option<String>> {
        match self {
            Self::String(value) => Ok(Some(value.to_string())),
            Self::Raw(raw) => raw.as_str().map(|s| s.map(|s| s.into_owned())),
            Self::Owned(owned) => owned.as_raw().as_str().map(|s| s.map(|s| s.into_owned())),
            _ => Ok(None),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn into_owned_static(self) -> QueryValue<'static> {
        match self {
            Self::Raw(raw) => QueryValue::Owned(raw.to_owned()),
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

    #[allow(dead_code)]
    pub(crate) fn from_static(value: QueryValue<'static>) -> Self {
        match value {
            QueryValue::Raw(raw) => Self::Owned(raw.to_owned()),
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

    fn into_jsonb_value(self) -> crate::error::Result<Value<'static>> {
        match self {
            Self::Null => Ok(Value::Null),
            Self::Bool(value) => Ok(Value::Bool(value)),
            Self::Number(value) => Ok(Value::Number(value)),
            Self::String(value) => Ok(Value::String(Cow::Owned(value.into_owned()))),
            Self::Array(values) => values
                .iter()
                .cloned()
                .map(QueryValue::into_jsonb_value)
                .collect::<crate::error::Result<Vec<_>>>()
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
                    crate::core::JsonbItemType::Null => 0,
                    crate::core::JsonbItemType::Boolean => 1,
                    crate::core::JsonbItemType::Number => 2,
                    crate::core::JsonbItemType::String => 3,
                    crate::core::JsonbItemType::Object(_) => 4,
                    crate::core::JsonbItemType::Array(_) => 5,
                    crate::core::JsonbItemType::Extension => 6,
                })
                .unwrap_or(6),
            Self::Owned(owned) => owned
                .as_raw()
                .jsonb_item_type()
                .map(|typ| match typ {
                    crate::core::JsonbItemType::Null => 0,
                    crate::core::JsonbItemType::Boolean => 1,
                    crate::core::JsonbItemType::Number => 2,
                    crate::core::JsonbItemType::String => 3,
                    crate::core::JsonbItemType::Object(_) => 4,
                    crate::core::JsonbItemType::Array(_) => 5,
                    crate::core::JsonbItemType::Extension => 6,
                })
                .unwrap_or(6),
        }
    }

    fn to_json_string(&self) -> String {
        match self {
            Self::Raw(raw) => raw.to_string(),
            Self::Owned(owned) => owned.as_raw().to_string(),
            value => value
                .clone()
                .into_owned_jsonb()
                .map(|owned| owned.as_raw().to_string())
                .unwrap_or_else(|_| "null".to_string()),
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
            _ => self.to_json_string().cmp(&other.to_json_string()),
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
