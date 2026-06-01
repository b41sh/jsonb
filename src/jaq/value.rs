use std::borrow::Cow;
use std::collections::BTreeMap;
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

fn raw_str_bytes(raw: RawJsonb<'_>) -> Option<&[u8]> {
    match JsonbItem::from_raw_jsonb(raw).ok()? {
        JsonbItem::String(value) => match value {
            Cow::Borrowed(value) => Some(value.as_bytes()),
            Cow::Owned(_) => None,
        },
        _ => None,
    }
}

fn as_isize(value: &QueryValue<'_>) -> Option<isize> {
    match as_number(value)? {
        Number::Int64(number) => isize::try_from(number).ok(),
        Number::UInt64(number) => isize::try_from(number).ok(),
        _ => None,
    }
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
            let Some(index_value) = as_isize(index) else {
                return Err(jaq_core::Error::index(
                    QueryValue::Owned(raw.to_owned()),
                    index.clone().into_owned_static(),
                ));
            };
            let Some(index) = abs_index(index_value, len) else {
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

fn raw_to_query_value(raw: RawJsonb<'_>) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    match raw.jsonb_item_type().map_err(jsonb_error)? {
        JsonbItemType::Array(_) => raw_values(raw).map(|values| QueryValue::Array(Rc::new(values))),
        JsonbItemType::Object(_) => raw_key_values(raw).map(|values| {
            QueryValue::Object(Rc::new(values.into_iter().collect::<BTreeMap<_, _>>()))
        }),
        _ => JsonbItem::from_raw_jsonb(raw)
            .map_err(jsonb_error)
            .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
            .map(QueryValue::into_owned_static),
    }
}

fn materialize(value: QueryValue<'_>) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    match value {
        QueryValue::Raw(raw) => raw_to_query_value(raw),
        QueryValue::Owned(owned) => raw_to_query_value(owned.as_raw()),
        value => Ok(value.into_owned_static()),
    }
}

fn materialize_current<'a>(value: QueryValue<'a>) -> ValR<QueryValue<'a>> {
    materialize(value).map(QueryValue::from_static)
}

fn range_from_object<'a, 'b>(index: &'b QueryValue<'a>) -> Option<Range<&'b QueryValue<'a>>> {
    let QueryValue::Object(object) = index else {
        return None;
    };
    let start = object.get(&QueryValue::from("start".to_string()));
    let end = object.get(&QueryValue::from("end".to_string()));
    Some(start..end)
}

fn into_array<'a>(value: QueryValue<'a>) -> ValR<Rc<Vec<QueryValue<'a>>>, QueryValue<'a>> {
    match materialize_current(value)? {
        QueryValue::Array(values) => Ok(values),
        value => Err(jaq_core::Error::typ(value, "array")),
    }
}

fn into_string<'a>(value: QueryValue<'a>) -> ValR<String, QueryValue<'a>> {
    match materialize_current(value)? {
        QueryValue::String(value) => Ok(value.into_owned()),
        value => Err(jaq_core::Error::typ(value, "string")),
    }
}

fn repeat_string<'a>(value: Cow<'a, str>, count: Number) -> ValR<QueryValue<'a>> {
    let Some(count) = count.as_i64() else {
        return Err(jaq_core::Error::typ(QueryValue::Number(count), "integer"));
    };
    if count <= 0 {
        return Ok(QueryValue::Null);
    }
    let count = usize::try_from(count).map_err(str_error)?;
    Ok(QueryValue::String(Cow::Owned(value.repeat(count))))
}

fn split_string<'a>(value: Cow<'a, str>, separator: Cow<'a, str>) -> QueryValue<'a> {
    let values = if value.is_empty() {
        Vec::new()
    } else if separator.is_empty() {
        value
            .chars()
            .map(|value| QueryValue::String(Cow::Owned(value.to_string())))
            .collect()
    } else {
        value
            .split(separator.as_ref())
            .map(|value| QueryValue::String(Cow::Owned(value.to_string())))
            .collect()
    };
    QueryValue::Array(Rc::new(values))
}

fn merge_object<'a>(
    left: &mut BTreeMap<QueryValue<'a>, QueryValue<'a>>,
    right: &BTreeMap<QueryValue<'a>, QueryValue<'a>>,
) -> ValR<(), QueryValue<'a>> {
    for (key, right_value) in right {
        let right_value = materialize_current(right_value.clone())?;
        match left.get_mut(key) {
            Some(left_value) => {
                let left_materialized = materialize_current(left_value.clone())?;
                match (left_materialized, right_value) {
                    (QueryValue::Object(mut left_object), QueryValue::Object(right_object)) => {
                        merge_object(Rc::make_mut(&mut left_object), &right_object)?;
                        *left_value = QueryValue::Object(left_object);
                    }
                    (_, right_value) => *left_value = right_value,
                }
            }
            None => {
                left.insert(key.clone(), right_value);
            }
        }
    }
    Ok(())
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
                let Some(index_value) = as_isize(index) else {
                    return Err(jaq_core::Error::index(Self::Array(values), index.clone()));
                };
                let Some(index) = abs_index(index_value, values.len()) else {
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
        f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        match materialize_current(self)? {
            Self::Array(values) => {
                let iter = values.iter().cloned().flat_map(f);
                Ok(Self::Array(Rc::new(iter.collect::<Result<_, _>>()?)))
            }
            Self::Object(values) => {
                let mut result = BTreeMap::new();
                for (key, value) in values.iter() {
                    if let Some(value) = f(value.clone()).next().transpose()? {
                        result.insert(key.clone(), value);
                    }
                }
                Ok(Self::Object(Rc::new(result)))
            }
            value => opt.fail(value, |value| {
                jaq_core::Exn::from(jaq_core::Error::typ(value, "iterable (array or object)"))
            }),
        }
    }

    fn map_index<'b, I: Iterator<Item = ValX<'b, Self>>>(
        self,
        index: &Self,
        opt: Opt,
        f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        let self_value = materialize_current(self)?;
        if matches!(self_value, Self::String(_) | Self::Array(_)) {
            if let Some(range) = range_from_object(index) {
                return self_value.map_range(range, opt, f);
            }
        };

        match self_value {
            Self::Object(mut values) => {
                let values = Rc::make_mut(&mut values);
                if let Some(existing) = values.remove(index) {
                    if let Some(value) = f(existing).next().transpose()? {
                        values.insert(index.clone(), value);
                    }
                } else if let Some(value) = f(Self::Null).next().transpose()? {
                    values.insert(index.clone(), value);
                }
                Ok(Self::Object(values.clone().into()))
            }
            Self::Array(mut values) => {
                let Some(index) = as_isize(index).and_then(|index| abs_index(index, values.len()))
                else {
                    return opt.fail(Self::Array(values), |_| {
                        jaq_core::Exn::from(jaq_core::Error::str(format!(
                            "index {index} out of bounds"
                        )))
                    });
                };
                let values = Rc::make_mut(&mut values);
                let value = values.remove(index);
                if let Some(value) = f(value).next().transpose()? {
                    values.insert(index, value);
                }
                Ok(Self::Array(values.clone().into()))
            }
            value => opt.fail(value, |value| {
                jaq_core::Exn::from(jaq_core::Error::typ(value, "iterable (array or object)"))
            }),
        }
    }

    fn map_range<'b, I: Iterator<Item = ValX<'b, Self>>>(
        self,
        range: Range<&Self>,
        opt: Opt,
        f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        match materialize_current(self)? {
            Self::Array(mut values) => {
                let start = match range_bound(range.start, values.len(), 0) {
                    Ok(start) => start,
                    Err(error) => {
                        return opt.fail(Self::Array(values), |_| jaq_core::Exn::from(error))
                    }
                };
                let end = match range_bound(range.end, values.len(), values.len()) {
                    Ok(end) => end,
                    Err(error) => {
                        return opt.fail(Self::Array(values), |_| jaq_core::Exn::from(error))
                    }
                };
                let take = end.saturating_sub(start);
                let selected = Self::Array(Rc::new(
                    values.iter().skip(start).take(take).cloned().collect(),
                ));
                let replacement = match f(selected).next().transpose()? {
                    Some(value) => into_array(value).map_err(jaq_core::Exn::from)?,
                    None => Rc::new(Vec::new()),
                };
                Rc::make_mut(&mut values).splice(start..start + take, replacement.iter().cloned());
                Ok(Self::Array(values))
            }
            Self::String(value) => {
                let chars = value.chars().collect::<Vec<_>>();
                let start = match range_bound(range.start, chars.len(), 0) {
                    Ok(start) => start,
                    Err(error) => {
                        return opt.fail(Self::String(value), |_| jaq_core::Exn::from(error))
                    }
                };
                let end = match range_bound(range.end, chars.len(), chars.len()) {
                    Ok(end) => end,
                    Err(error) => {
                        return opt.fail(Self::String(value), |_| jaq_core::Exn::from(error))
                    }
                };
                let take = end.saturating_sub(start);
                let selected = Self::String(Cow::Owned(
                    chars.iter().skip(start).take(take).collect::<String>(),
                ));
                let replacement = match f(selected).next().transpose()? {
                    Some(value) => into_string(value).map_err(jaq_core::Exn::from)?,
                    None => String::new(),
                };
                let result = chars
                    .iter()
                    .take(start)
                    .chain(replacement.chars().collect::<Vec<_>>().iter())
                    .chain(chars.iter().skip(start + take))
                    .collect::<String>();
                Ok(Self::String(Cow::Owned(result)))
            }
            value => opt.fail(value, |value| {
                jaq_core::Exn::from(jaq_core::Error::typ(value, "rangeable (array or string)"))
            }),
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
        match materialize_current(self.clone()).unwrap_or(self) {
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
            Self::Raw(raw) => raw_str_bytes(*raw),
            Self::Owned(owned) => raw_str_bytes(owned.as_raw()),
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
                _ => match (materialize_current(left)?, materialize_current(right)?) {
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
            _ => match (materialize_current(self)?, materialize_current(rhs)?) {
                (Self::Array(mut left), Self::Array(right)) => {
                    let right = right.iter().collect::<std::collections::BTreeSet<_>>();
                    Rc::make_mut(&mut left).retain(|value| !right.contains(value));
                    Ok(Self::Array(left))
                }
                (left, right) => Err(jaq_core::Error::math(left, jaq_core::ops::Math::Sub, right)),
            },
        }
    }
}

impl<'a> Mul for QueryValue<'a> {
    type Output = ValR<Self>;

    fn mul(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right)) => left.mul(right).map(Self::Number).map_err(jsonb_error),
            _ => match (materialize_current(self)?, materialize_current(rhs)?) {
                (Self::String(value), Self::Number(count))
                | (Self::Number(count), Self::String(value)) => repeat_string(value, count),
                (Self::Object(mut left), Self::Object(right)) => {
                    merge_object(Rc::make_mut(&mut left), &right)?;
                    Ok(Self::Object(left))
                }
                (left, right) => Err(jaq_core::Error::math(left, jaq_core::ops::Math::Mul, right)),
            },
        }
    }
}

impl<'a> Div for QueryValue<'a> {
    type Output = ValR<Self>;

    fn div(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right))
                if matches!(left, Number::Float64(_)) || matches!(right, Number::Float64(_)) =>
            {
                Ok(Self::Number(Number::Float64(
                    left.as_f64() / right.as_f64(),
                )))
            }
            (Some(left), Some(right)) => left.div(right).map(Self::Number).map_err(jsonb_error),
            _ => match (materialize_current(self)?, materialize_current(rhs)?) {
                (Self::String(left), Self::String(right)) => Ok(split_string(left, right)),
                (left, right) => Err(jaq_core::Error::math(left, jaq_core::ops::Math::Div, right)),
            },
        }
    }
}

impl<'a> Rem for QueryValue<'a> {
    type Output = ValR<Self>;

    fn rem(self, rhs: Self) -> Self::Output {
        match (as_number(&self), as_number(&rhs)) {
            (Some(left), Some(right))
                if matches!(left, Number::Float64(_)) || matches!(right, Number::Float64(_)) =>
            {
                Ok(Self::Number(Number::Float64(
                    left.as_f64() % right.as_f64(),
                )))
            }
            (Some(left), Some(right)) => left.rem(right).map(Self::Number).map_err(jsonb_error),
            _ => {
                let left = materialize_current(self)?;
                let right = materialize_current(rhs)?;
                Err(jaq_core::Error::math(left, jaq_core::ops::Math::Rem, right))
            }
        }
    }
}

impl<'a> Neg for QueryValue<'a> {
    type Output = ValR<Self>;

    fn neg(self) -> Self::Output {
        match as_number(&self) {
            Some(number) => number.neg().map(Self::Number).map_err(jsonb_error),
            None => Err(jaq_core::Error::typ(materialize_current(self)?, "number")),
        }
    }
}

#[cfg(test)]
mod tests {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::ValT;
    use jaq_core::{unwrap_valr, Compiler, Ctx, Vars};

    use crate::core::QueryValue;
    use crate::jaq::{defs, funs, JsonbData};
    use crate::OwnedJsonb;

    fn query_value(json: &str) -> QueryValue<'static> {
        QueryValue::Owned(json.parse::<OwnedJsonb>().unwrap())
    }

    fn run_filter(filter: &'static str, input: &str) -> Vec<String> {
        let arena = Arena::default();
        let loader = Loader::new(jaq_core::defs().chain(jaq_std::defs()).chain(defs()));
        let modules = loader
            .load(
                &arena,
                File {
                    path: (),
                    code: filter,
                },
            )
            .unwrap();
        let filter = Compiler::default()
            .with_funs(
                jaq_core::funs::<JsonbData>()
                    .chain(jaq_std::funs::<JsonbData>())
                    .chain(funs::<JsonbData>()),
            )
            .compile(modules)
            .unwrap();

        let input = QueryValue::Owned(input.parse::<OwnedJsonb>().unwrap());
        let ctx = Ctx::<JsonbData>::new(&filter.lut, Vars::new([]));
        filter
            .id
            .run((ctx, input))
            .map(unwrap_valr)
            .map(|value| {
                value
                    .unwrap()
                    .into_owned_jsonb()
                    .unwrap()
                    .as_raw()
                    .to_string()
            })
            .collect()
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

    #[test]
    fn update_object_field() {
        assert_eq!(
            run_filter(".a |= . + 1", r#"{"a":1,"b":2}"#),
            [r#"{"a":2,"b":2}"#]
        );
    }

    #[test]
    fn delete_object_field() {
        assert_eq!(run_filter("del(.a)", r#"{"a":1,"b":2}"#), [r#"{"b":2}"#]);
    }

    #[test]
    fn delete_array_index() {
        assert_eq!(run_filter("del(.[1])", "[1,2,3]"), ["[1,3]"]);
    }

    #[test]
    fn update_array_values() {
        assert_eq!(run_filter(".[] |= . + 1", "[1,2]"), ["[2,3]"]);
    }

    #[test]
    fn update_array_range() {
        assert_eq!(run_filter(".[1:3] |= [9]", "[1,2,3,4]"), ["[1,9,4]"]);
    }

    #[test]
    fn update_string_range() {
        assert_eq!(run_filter(r#".[1:3] |= "X""#, r#""abcd""#), [r#""aXd""#]);
    }

    #[test]
    fn update_array_with_multiple_outputs() {
        assert_eq!(run_filter(".[] |= (., . + 1)", "[1,3]"), ["[1,2,3,4]"]);
    }

    #[test]
    fn update_nested_path() {
        assert_eq!(
            run_filter(".a.b += 1", r#"{"a":{"b":1},"c":2}"#),
            [r#"{"a":{"b":2},"c":2}"#]
        );
    }

    #[test]
    fn optional_out_of_bounds_update_keeps_input() {
        assert_eq!(run_filter(".[3]? |= . + 1", "[1,2]"), ["[1,2]"]);
    }

    #[test]
    fn subtract_arrays() {
        assert_eq!(
            run_filter(".a - .b", r#"{"a":[1,2,3,2],"b":[2]}"#),
            ["[1,3]"]
        );
    }

    #[test]
    fn multiply_string_by_number() {
        assert_eq!(run_filter(".s * 3", r#"{"s":"ab"}"#), [r#""ababab""#]);
        assert_eq!(run_filter(".s * 0", r#"{"s":"ab"}"#), ["null"]);
    }

    #[test]
    fn split_string_by_string() {
        assert_eq!(
            run_filter(r#".s / ",""#, r#"{"s":"a,b,c"}"#),
            [r#"["a","b","c"]"#]
        );
        assert_eq!(
            run_filter(r#".s / """#, r#"{"s":"abc"}"#),
            [r#"["a","b","c"]"#]
        );
    }

    #[test]
    fn multiply_objects_merges_recursively() {
        assert_eq!(
            run_filter(
                ".a * .b",
                r#"{"a":{"x":{"left":1},"replace":1},"b":{"x":{"right":2},"replace":2}}"#
            ),
            [r#"{"replace":2,"x":{"left":1,"right":2}}"#]
        );
    }

    #[test]
    fn std_array_functions_accept_owned_jsonb_arrays() {
        assert_eq!(run_filter("reverse", "[1,2,3]"), ["[3,2,1]"]);
        assert_eq!(run_filter("sort", "[3,1,2]"), ["[1,2,3]"]);
    }

    #[test]
    fn std_string_functions_accept_owned_jsonb_strings() {
        assert_eq!(run_filter("explode", r#""ab""#), ["[97,98]"]);
        assert_eq!(run_filter("utf8bytelength", r#""你好""#), ["6"]);
        assert_eq!(run_filter("ascii_upcase", r#""ab""#), [r#""AB""#]);
        assert_eq!(run_filter(r#"ltrimstr("ab")"#, r#""abcd""#), [r#""cd""#]);
    }

    #[test]
    fn std_string_functions_accept_raw_child_strings() {
        assert_eq!(
            run_filter(r#".s | startswith("ab")"#, r#"{"s":"abcd"}"#),
            ["true"]
        );
        assert_eq!(
            run_filter(r#".s | endswith("cd")"#, r#"{"s":"abcd"}"#),
            ["true"]
        );
    }
}
