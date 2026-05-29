use std::borrow::Cow;
use std::ops::{Add, Div, Mul, Neg, Rem, Sub};
use std::rc::Rc;

use jaq_core::path::Opt;
use jaq_core::val::{Range, ValR, ValX};

use crate::core::QueryValue;
use crate::core::{ArrayIterator, JsonbItem, JsonbItemType, ObjectIterator, ObjectValueIterator};
use crate::{Number, OwnedJsonb, RawJsonb};

fn as_number(value: &QueryValue<'_>) -> Option<Number> {
    match value {
        QueryValue::Number(number) => Some(number.clone()),
        QueryValue::Raw(raw) => raw.as_number().ok().flatten(),
        QueryValue::Owned(owned) => owned.as_raw().as_number().ok().flatten(),
        _ => None,
    }
}

fn as_key_string(value: &QueryValue<'_>) -> Option<String> {
    match value {
        QueryValue::String(value) => Some(value.to_string()),
        QueryValue::Raw(raw) => raw.as_str().ok().flatten().map(|value| value.into_owned()),
        QueryValue::Owned(owned) => owned
            .as_raw()
            .as_str()
            .ok()
            .flatten()
            .map(|value| value.into_owned()),
        _ => None,
    }
}

fn as_isize(value: &QueryValue<'_>) -> Option<isize> {
    as_number(value)
        .and_then(|number| number.as_i64())
        .and_then(|number| isize::try_from(number).ok())
}

fn abs_index(index: isize, len: usize) -> Option<usize> {
    if index >= 0 {
        usize::try_from(index).ok().filter(|index| *index < len)
    } else {
        len.checked_sub(index.unsigned_abs())
    }
}

fn range_bound<'a>(
    bound: Option<&QueryValue<'a>>,
    len: usize,
    default: usize,
) -> ValR<usize, QueryValue<'static>> {
    match bound {
        None | Some(QueryValue::Null) => Ok(default),
        Some(value) => {
            let index = as_isize(value).ok_or_else(|| {
                jaq_core::Error::typ(value.clone().into_owned_static(), "integer")
            })?;
            Ok(if index >= 0 {
                usize::try_from(index).unwrap_or(usize::MAX).min(len)
            } else {
                len.saturating_sub(index.unsigned_abs())
            })
        }
    }
}

fn string_range<'a>(
    value: Cow<'a, str>,
    range: Range<&QueryValue<'a>>,
) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    let chars: Vec<_> = value.chars().collect();
    let start = range_bound(range.start, chars.len(), 0)?;
    let end = range_bound(range.end, chars.len(), chars.len())?;
    let take = end.saturating_sub(start);
    Ok(QueryValue::String(Cow::Owned(
        chars.into_iter().skip(start).take(take).collect(),
    )))
}

fn item_to_query_value<'a>(item: JsonbItem<'a>) -> crate::error::Result<QueryValue<'a>> {
    match item {
        JsonbItem::Null => Ok(QueryValue::Null),
        JsonbItem::Boolean(value) => Ok(QueryValue::Bool(value)),
        JsonbItem::Number(value) => value.as_number().map(QueryValue::Number),
        JsonbItem::String(value) => Ok(QueryValue::String(Cow::Owned(value.into_owned()))),
        JsonbItem::Raw(raw) => Ok(QueryValue::Owned(raw.to_owned())),
        JsonbItem::Owned(owned) => Ok(QueryValue::Owned(owned)),
        JsonbItem::Extension(value) => {
            OwnedJsonb::from_item(JsonbItem::Extension(value)).map(QueryValue::Owned)
        }
    }
}

fn raw_values(raw: RawJsonb<'_>) -> ValR<Vec<QueryValue<'static>>, QueryValue<'static>> {
    match raw.jsonb_item_type().map_err(jsonb_error)? {
        JsonbItemType::Array(_) => {
            let iter = ArrayIterator::new(raw)
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as iterable (array or object)"))?;
            iter.map(|item| {
                item.map_err(jsonb_error)
                    .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                    .map(QueryValue::into_owned_static)
            })
            .collect()
        }
        JsonbItemType::Object(_) => {
            let iter = ObjectValueIterator::new(raw)
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as iterable (array or object)"))?;
            iter.map(|item| {
                item.map_err(jsonb_error)
                    .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                    .map(QueryValue::into_owned_static)
            })
            .collect()
        }
        _ => Err(jaq_core::Error::typ(
            QueryValue::Owned(raw.to_owned()),
            "iterable (array or object)",
        )),
    }
}

fn raw_key_values<'a>(
    raw: RawJsonb<'a>,
) -> ValR<Vec<(QueryValue<'static>, QueryValue<'static>)>, QueryValue<'static>> {
    match raw.jsonb_item_type().map_err(jsonb_error)? {
        JsonbItemType::Array(_) => raw_values(raw).map(|values| {
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| (QueryValue::from(index), value))
                .collect()
        }),
        JsonbItemType::Object(_) => {
            let iter = ObjectIterator::new(raw)
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as iterable (array or object)"))?;
            iter.map(|item| {
                item.map_err(jsonb_error).and_then(|(key, value)| {
                    item_to_query_value(value)
                        .map_err(jsonb_error)
                        .map(|value| (QueryValue::from(key.to_string()), value.into_owned_static()))
                })
            })
            .collect()
        }
        _ => Err(jaq_core::Error::typ(
            QueryValue::Owned(raw.to_owned()),
            "iterable (array or object)",
        )),
    }
}

fn raw_index<'a>(
    raw: RawJsonb<'a>,
    index: &QueryValue<'a>,
) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    match raw.jsonb_item_type().map_err(jsonb_error)? {
        JsonbItemType::Array(len) => {
            let Some(index) = as_isize(index).and_then(|index| abs_index(index, len)) else {
                return Ok(QueryValue::Null);
            };
            let mut iter = ArrayIterator::new(raw)
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as array"))?;
            match iter.nth(index) {
                Some(item) => item
                    .map_err(jsonb_error)
                    .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                    .map(QueryValue::into_owned_static),
                None => Ok(QueryValue::Null),
            }
        }
        JsonbItemType::Object(_) => {
            let Some(key) = as_key_string(index) else {
                return Ok(QueryValue::Null);
            };
            let iter = ObjectIterator::new(raw)
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as object"))?;
            for item in iter {
                let (item_key, value) = item.map_err(jsonb_error)?;
                if item_key == key {
                    return item_to_query_value(value)
                        .map_err(jsonb_error)
                        .map(QueryValue::into_owned_static);
                }
            }
            Ok(QueryValue::Null)
        }
        JsonbItemType::Null => Ok(QueryValue::Null),
        _ => Err(jaq_core::Error::index(
            QueryValue::Owned(raw.to_owned()),
            index.clone().into_owned_static(),
        )),
    }
}

fn raw_range<'a>(
    raw: RawJsonb<'a>,
    range: Range<&QueryValue<'a>>,
) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    match raw.jsonb_item_type().map_err(jsonb_error)? {
        JsonbItemType::Array(len) => {
            let start = range_bound(range.start, len, 0)?;
            let end = range_bound(range.end, len, len)?;
            let take = end.saturating_sub(start);
            let iter = ArrayIterator::new(raw)
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as array"))?;
            iter.skip(start)
                .take(take)
                .map(|item| {
                    item.map_err(jsonb_error)
                        .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                        .map(QueryValue::into_owned_static)
                })
                .collect::<ValR<Vec<_>, _>>()
                .map(|values| QueryValue::Array(Rc::new(values)))
        }
        JsonbItemType::String => {
            let value = raw
                .as_str()
                .map_err(jsonb_error)?
                .ok_or_else(|| str_error("cannot use value as string"))?;
            string_range(value, range)
        }
        _ => Err(jaq_core::Error::typ(
            QueryValue::Owned(raw.to_owned()),
            "rangeable (array or string)",
        )),
    }
}

fn into_static_iter<T>(items: Vec<T>) -> Box<dyn Iterator<Item = T> + 'static> {
    let iter: Box<dyn Iterator<Item = T>> = Box::new(items.into_iter());
    // All values placed in these iterators are converted through
    // `QueryValue::into_owned_static`, so no borrowed JSONB data is retained.
    unsafe {
        std::mem::transmute::<Box<dyn Iterator<Item = T>>, Box<dyn Iterator<Item = T> + 'static>>(
            iter,
        )
    }
}

fn str_error<'a>(message: impl ToString) -> jaq_core::Error<QueryValue<'a>> {
    jaq_core::Error::str(message)
}

fn jsonb_error<'a>(error: crate::Error) -> jaq_core::Error<QueryValue<'a>> {
    str_error(error)
}

impl<'a> jaq_core::ValT for QueryValue<'a> {
    fn from_num(number: &str) -> ValR<Self> {
        if let Ok(value) = number.parse::<i64>() {
            return Ok(Self::Number(Number::Int64(value)));
        }
        if let Ok(value) = number.parse::<u64>() {
            return Ok(Self::Number(Number::UInt64(value)));
        }
        number
            .parse::<f64>()
            .map(|value| Self::Number(Number::Float64(value)))
            .map_err(str_error)
    }

    fn from_map<I: IntoIterator<Item = (Self, Self)>>(iter: I) -> ValR<Self> {
        Ok(Self::Object(Rc::new(iter.into_iter().collect())))
    }

    fn key_values(self) -> jaq_core::box_iter::BoxIter<'static, ValR<(Self, Self), Self>> {
        let values = match self {
            Self::Raw(raw) => raw_key_values(raw),
            Self::Owned(owned) => raw_key_values(owned.as_raw()),
            Self::Array(values) => Ok(values
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, value)| (QueryValue::from(index), value.into_owned_static()))
                .collect()),
            Self::Object(values) => Ok(values
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone().into_owned_static(),
                        value.clone().into_owned_static(),
                    )
                })
                .collect()),
            value => Err(jaq_core::Error::typ(
                value.into_owned_static(),
                "iterable (array or object)",
            )),
        };
        match values {
            Ok(values) => into_static_iter(values.into_iter().map(Ok).collect()),
            Err(error) => into_static_iter(vec![Err(error)]),
        }
    }

    fn values(self) -> Box<dyn Iterator<Item = ValR<Self>>> {
        let values = match self {
            Self::Raw(raw) => raw_values(raw),
            Self::Owned(owned) => raw_values(owned.as_raw()),
            Self::Array(values) => Ok(values
                .iter()
                .cloned()
                .map(QueryValue::into_owned_static)
                .collect()),
            Self::Object(values) => Ok(values
                .values()
                .cloned()
                .map(QueryValue::into_owned_static)
                .collect()),
            value => Err(jaq_core::Error::typ(
                value.into_owned_static(),
                "iterable (array or object)",
            )),
        };
        match values {
            Ok(values) => into_static_iter(values.into_iter().map(Ok).collect()),
            Err(error) => into_static_iter(vec![Err(error)]),
        }
    }

    fn index(self, index: &Self) -> ValR<Self> {
        match self {
            Self::Raw(raw) => raw_index(raw, index).map(QueryValue::from_static),
            Self::Owned(owned) => raw_index(owned.as_raw(), index).map(QueryValue::from_static),
            Self::Null => Ok(Self::Null),
            Self::Array(values) => {
                let Some(index) = as_isize(index).and_then(|index| abs_index(index, values.len()))
                else {
                    return Ok(Self::Null);
                };
                Ok(values
                    .get(index)
                    .cloned()
                    .map(QueryValue::into_owned_static)
                    .map(QueryValue::from_static)
                    .unwrap_or(Self::Null))
            }
            Self::Object(values) => Ok(values.get(index).cloned().unwrap_or(Self::Null)),
            value => Err(jaq_core::Error::index(value, index.clone())),
        }
    }

    fn range(self, range: Range<&Self>) -> ValR<Self> {
        match self {
            Self::Raw(raw) => raw_range(raw, range).map(QueryValue::from_static),
            Self::Owned(owned) => raw_range(owned.as_raw(), range).map(QueryValue::from_static),
            Self::Array(values) => {
                let start = range_bound(range.start, values.len(), 0)?;
                let end = range_bound(range.end, values.len(), values.len())?;
                let take = end.saturating_sub(start);
                Ok(Self::Array(Rc::new(
                    values
                        .iter()
                        .skip(start)
                        .take(take)
                        .cloned()
                        .map(QueryValue::into_owned_static)
                        .map(QueryValue::from_static)
                        .collect(),
                )))
            }
            Self::String(value) => string_range(value, range).map(QueryValue::from_static),
            value => Err(jaq_core::Error::typ(value, "rangeable (array or string)")),
        }
    }

    fn map_values<'b, I: Iterator<Item = ValX<'b, Self>>>(
        self,
        opt: Opt,
        _f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        match opt {
            Opt::Optional => Ok(self),
            Opt::Essential => {
                Err(str_error("jsonb jaq value updates are not implemented yet").into())
            }
        }
    }

    fn map_index<'b, I: Iterator<Item = ValX<'b, Self>>>(
        self,
        _index: &Self,
        opt: Opt,
        _f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        match opt {
            Opt::Optional => Ok(self),
            Opt::Essential => {
                Err(str_error("jsonb jaq index updates are not implemented yet").into())
            }
        }
    }

    fn map_range<'b, I: Iterator<Item = ValX<'b, Self>>>(
        self,
        _range: Range<&Self>,
        opt: Opt,
        _f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        match opt {
            Opt::Optional => Ok(self),
            Opt::Essential => {
                Err(str_error("jsonb jaq range updates are not implemented yet").into())
            }
        }
    }

    fn as_bool(&self) -> bool {
        !matches!(self, Self::Null | Self::Bool(false))
    }

    fn into_string(self) -> Self {
        match self {
            Self::String(_) => self,
            value => Self::String(Cow::Owned(value.to_string())),
        }
    }
}

impl<'a> jaq_std::ValT for QueryValue<'a> {
    fn into_seq<S: FromIterator<Self>>(self) -> Result<S, Self> {
        match self {
            Self::Array(values) => Ok(values.iter().cloned().collect()),
            value => Err(value),
        }
    }

    fn is_int(&self) -> bool {
        as_number(self).is_some_and(|number| number.as_i64().is_some() || number.as_u64().is_some())
    }

    fn as_isize(&self) -> Option<isize> {
        as_isize(self)
    }

    fn as_f64(&self) -> Option<f64> {
        as_number(self).map(|number| number.as_f64())
    }

    fn is_utf8_str(&self) -> bool {
        matches!(self, Self::String(_))
            || matches!(self, Self::Raw(raw) if raw.as_str().ok().flatten().is_some())
            || matches!(self, Self::Owned(owned) if owned.as_raw().as_str().ok().flatten().is_some())
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    fn as_sub_str(&self, sub: &[u8]) -> Self {
        match std::str::from_utf8(sub) {
            Ok(value) => Self::String(Cow::Owned(value.to_string())),
            Err(_) => Self::String(Cow::Owned(String::from_utf8_lossy(sub).into_owned())),
        }
    }

    fn from_utf8_bytes(bytes: impl AsRef<[u8]> + Send + 'static) -> Self {
        Self::String(Cow::Owned(
            String::from_utf8_lossy(bytes.as_ref()).into_owned(),
        ))
    }
}

impl<'a> Add for QueryValue<'a> {
    type Output = ValR<Self>;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Null, value) | (value, Self::Null) => Ok(value),
            (left, right) => match (as_number(&left), as_number(&right)) {
                (Some(left), Some(right)) => left.add(right).map(Self::Number).map_err(jsonb_error),
                _ => match (left, right) {
                    (Self::String(left), Self::String(right)) => Ok(Self::String(Cow::Owned(
                        format!("{}{}", left.as_ref(), right.as_ref()),
                    ))),
                    (Self::Array(left), Self::Array(right)) => Ok(Self::Array(Rc::new(
                        left.iter().chain(right.iter()).cloned().collect(),
                    ))),
                    (Self::Object(mut left), Self::Object(right)) => {
                        Rc::make_mut(&mut left).extend(
                            right
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone())),
                        );
                        Ok(Self::Object(left))
                    }
                    (left, right) => {
                        Err(jaq_core::Error::math(left, jaq_core::ops::Math::Add, right))
                    }
                },
            },
        }
    }
}

impl<'a> Sub for QueryValue<'a> {
    type Output = ValR<Self>;

    fn sub(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right)) => left.sub(right).map(Self::Number).map_err(jsonb_error),
            _ => Err(str_error(
                "jsonb jaq subtraction is not implemented for these values",
            )),
        }
    }
}

impl<'a> Mul for QueryValue<'a> {
    type Output = ValR<Self>;

    fn mul(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right)) => left.mul(right).map(Self::Number).map_err(jsonb_error),
            _ => Err(str_error(
                "jsonb jaq multiplication is not implemented for these values",
            )),
        }
    }
}

impl<'a> Div for QueryValue<'a> {
    type Output = ValR<Self>;

    fn div(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right)) => left.div(right).map(Self::Number).map_err(jsonb_error),
            _ => Err(str_error(
                "jsonb jaq division is not implemented for these values",
            )),
        }
    }
}

impl<'a> Rem for QueryValue<'a> {
    type Output = ValR<Self>;

    fn rem(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right)) => left.rem(right).map(Self::Number).map_err(jsonb_error),
            _ => Err(str_error(
                "jsonb jaq remainder is not implemented for these values",
            )),
        }
    }
}

impl<'a> Neg for QueryValue<'a> {
    type Output = ValR<Self>;

    fn neg(self) -> Self::Output {
        match as_number(&self) {
            Some(number) => number.neg().map(Self::Number).map_err(jsonb_error),
            None => Err(str_error(
                "jsonb jaq negation is not implemented for this value",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use jaq_core::ValT;

    use crate::core::QueryValue;
    use crate::OwnedJsonb;

    fn query_value(json: &str) -> QueryValue<'static> {
        QueryValue::Owned(json.parse::<OwnedJsonb>().unwrap())
    }

    #[test]
    fn index_raw_array_and_object() {
        let value = query_value(r#"{"items":[{"id":1},{"id":2}]}"#);
        let items = value.index(&QueryValue::from("items".to_string())).unwrap();
        let second = items.index(&QueryValue::from(1isize)).unwrap();
        let id = second.index(&QueryValue::from("id".to_string())).unwrap();

        assert_eq!(id.into_owned_jsonb().unwrap().as_raw().to_string(), "2");
    }

    #[test]
    fn iterate_raw_array_values() {
        let value = query_value("[1,2,3]");
        let values = value
            .values()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .map(|value| value.into_owned_jsonb().unwrap().as_raw().to_string())
            .collect::<Vec<_>>();

        assert_eq!(values, ["1", "2", "3"]);
    }

    #[test]
    fn range_raw_array() {
        let value = query_value("[1,2,3,4]");
        let range = value
            .range(Some(&QueryValue::from(1isize))..Some(&QueryValue::from(3isize)))
            .unwrap();

        assert_eq!(
            range.into_owned_jsonb().unwrap().as_raw().to_string(),
            "[2,3]"
        );
    }
}
