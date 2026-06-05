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
use std::collections::BTreeMap;
use std::ops::Add;
use std::ops::Div;
use std::ops::Mul;
use std::ops::Neg;
use std::ops::Rem;
use std::ops::Sub;
use std::rc::Rc;

use jaq_core::path::Opt;
use jaq_core::val::Range;
use jaq_core::val::ValR;
use jaq_core::val::ValX;

use crate::core::QueryValue;
use crate::parse_value;
use crate::Number;
use crate::Value;

use super::access::abs_index;
use super::access::array_subsequence_indices;
use super::access::bytes_range;
use super::access::jsonb_error;
use super::access::range_bound;
use super::access::str_error;
use super::access::string_range;

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

fn repeat_bytes<'a>(value: Cow<'a, [u8]>, count: Number) -> ValR<QueryValue<'a>> {
    let Some(count) = count.as_i64() else {
        return Err(jaq_core::Error::typ(QueryValue::Number(count), "integer"));
    };
    if count <= 0 {
        return Ok(QueryValue::Null);
    }
    let count = usize::try_from(count).map_err(str_error)?;
    Ok(QueryValue::Bytes(Cow::Owned(value.repeat(count))))
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

fn split_bytes<'a>(value: Cow<'a, [u8]>, separator: Cow<'a, [u8]>) -> QueryValue<'a> {
    fn find_bytes(value: &[u8], separator: &[u8]) -> Option<usize> {
        value
            .windows(separator.len())
            .position(|window| window == separator)
    }

    let values = if value.is_empty() {
        Vec::new()
    } else if separator.is_empty() {
        value
            .iter()
            .map(|value| QueryValue::Bytes(Cow::Owned(vec![*value])))
            .collect()
    } else {
        let mut values = Vec::new();
        let mut remaining = value.as_ref();
        while let Some(index) = find_bytes(remaining, separator.as_ref()) {
            values.push(QueryValue::Bytes(Cow::Owned(remaining[..index].to_vec())));
            remaining = &remaining[index + separator.len()..];
        }
        values.push(QueryValue::Bytes(Cow::Owned(remaining.to_vec())));
        values
    };
    QueryValue::Array(Rc::new(values))
}

fn integer_number(value: &Number) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(|value| value as i128))
}

fn merge_object<'a>(
    left: &mut BTreeMap<QueryValue<'a>, QueryValue<'a>>,
    right: &BTreeMap<QueryValue<'a>, QueryValue<'a>>,
) -> ValR<(), QueryValue<'a>> {
    for (key, right_value) in right {
        let right_value = right_value.clone().materialize_current()?;
        match left.get_mut(key) {
            Some(left_value) => {
                let left_materialized = left_value.clone().materialize_current()?;
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

impl<'a> jaq_core::ValT for QueryValue<'a> {
    fn from_num(number: &str) -> ValR<Self> {
        match parse_value(number.as_bytes()).map_err(jsonb_error)? {
            Value::Number(number) => Ok(Self::Number(number)),
            _ => Err(str_error("number expected")),
        }
    }

    fn from_map<I: IntoIterator<Item = (Self, Self)>>(iter: I) -> ValR<Self> {
        Ok(Self::Object(Rc::new(iter.into_iter().collect())))
    }

    fn key_values(self) -> jaq_core::box_iter::BoxIter<'static, ValR<(Self, Self), Self>> {
        let values = match self {
            Self::Raw(raw) => raw.raw_key_values(),
            Self::Owned(owned) => owned.as_raw().raw_key_values(),
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
            Self::Raw(raw) => raw.raw_values(),
            Self::Owned(owned) => owned.as_raw().raw_values(),
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
            Self::Raw(raw) => raw.raw_index(index).map(QueryValue::from_static),
            Self::Owned(owned) => owned.as_raw().raw_index(index).map(QueryValue::from_static),
            Self::Null => Ok(Self::Null),
            Self::Array(values) => {
                if let Some(range) = index.as_range_object_owned()? {
                    return Self::Array(values).range(range.start.as_ref()..range.end.as_ref());
                }

                if let Some(needle) = index.as_array_values_owned()? {
                    let values = values
                        .iter()
                        .cloned()
                        .map(QueryValue::into_owned_static)
                        .collect::<Vec<_>>();
                    return Ok(Self::from_static(array_subsequence_indices(
                        &values, &needle,
                    )));
                }

                let Some(index_value) = index.as_isize().ok().flatten() else {
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
            Self::String(value) => {
                if let Some(range) = index.as_range_object_owned()? {
                    return Self::String(value).range(range.start.as_ref()..range.end.as_ref());
                }
                Err(jaq_core::Error::index(Self::String(value), index.clone()))
            }
            Self::Bytes(value) => {
                if let Some(range) = index.as_range_object_owned()? {
                    return Self::Bytes(value).range(range.start.as_ref()..range.end.as_ref());
                }
                Err(jaq_core::Error::index(Self::Bytes(value), index.clone()))
            }
            Self::Object(values) => Ok(values.get(index).cloned().unwrap_or(Self::Null)),
            value => Err(jaq_core::Error::index(value, index.clone())),
        }
    }

    fn range(self, range: Range<&Self>) -> ValR<Self> {
        match self {
            Self::Raw(raw) => raw.raw_range(range).map(QueryValue::from_static),
            Self::Owned(owned) => owned.as_raw().raw_range(range).map(QueryValue::from_static),
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
            Self::Bytes(value) => bytes_range(value, range).map(QueryValue::from_static),
            value => Err(jaq_core::Error::typ(value, "rangeable (array or string)")),
        }
    }

    fn map_values<'b, I: Iterator<Item = ValX<'b, Self>>>(
        self,
        opt: Opt,
        f: impl Fn(Self) -> I,
    ) -> ValX<'b, Self> {
        match self.materialize_current()? {
            Self::Array(values) => {
                let iter = values.iter().cloned().flat_map(f);
                Ok(Self::Array(Rc::new(
                    iter.collect::<std::result::Result<_, _>>()?,
                )))
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
        let self_value = self.materialize_current()?;
        if matches!(
            self_value,
            Self::String(_) | Self::Bytes(_) | Self::Array(_)
        ) {
            if let Some(range) = index.as_range_object_owned()? {
                return self_value.map_range(range.start.as_ref()..range.end.as_ref(), opt, f);
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
                let Some(index) = index
                    .as_isize()
                    .ok()
                    .flatten()
                    .and_then(|index| abs_index(index, values.len()))
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
        match self.materialize_current()? {
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
                    Some(value) => value.into_array().map_err(jaq_core::Exn::from)?,
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
                    Some(value) => value.into_string_value().map_err(jaq_core::Exn::from)?,
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
            Self::Bytes(value) => {
                let start = match range_bound(range.start, value.len(), 0) {
                    Ok(start) => start,
                    Err(error) => {
                        return opt.fail(Self::Bytes(value), |_| jaq_core::Exn::from(error))
                    }
                };
                let end = match range_bound(range.end, value.len(), value.len()) {
                    Ok(end) => end,
                    Err(error) => {
                        return opt.fail(Self::Bytes(value), |_| jaq_core::Exn::from(error))
                    }
                };
                let take = end.saturating_sub(start);
                let mut bytes = value.into_owned();
                let selected = Self::Bytes(Cow::Owned(bytes[start..start + take].to_vec()));
                let replacement = match f(selected).next().transpose()? {
                    Some(value) => value.into_bytes_value().map_err(jaq_core::Exn::from)?,
                    None => Vec::new(),
                };
                bytes.splice(start..start + take, replacement);
                Ok(Self::Bytes(Cow::Owned(bytes)))
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
            Self::Bytes(value) => {
                Self::String(Cow::Owned(String::from_utf8_lossy(&value).into_owned()))
            }
            value => Self::String(Cow::Owned(value.to_string())),
        }
    }
}

impl<'a> jaq_std::ValT for QueryValue<'a> {
    fn into_seq<S: FromIterator<Self>>(self) -> std::result::Result<S, Self> {
        match self.clone().materialize_current().unwrap_or(self) {
            Self::Array(values) => Ok(values.iter().cloned().collect()),
            value => Err(value),
        }
    }

    fn is_int(&self) -> bool {
        self.as_number()
            .ok()
            .flatten()
            .is_some_and(|number| number.as_i64().is_some() || number.as_u64().is_some())
    }

    fn as_isize(&self) -> Option<isize> {
        QueryValue::as_isize(self).ok().flatten()
    }

    fn as_f64(&self) -> Option<f64> {
        self.as_number()
            .ok()
            .flatten()
            .map(|number| number.as_f64())
    }

    fn is_utf8_str(&self) -> bool {
        self.as_string().ok().flatten().is_some()
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(value) => Some(value.as_bytes()),
            Self::Bytes(value) => Some(value.as_ref()),
            Self::Raw(raw) => raw.raw_bytes(),
            Self::Owned(owned) => owned.as_raw().raw_bytes(),
            _ => None,
        }
    }

    fn as_sub_str(&self, sub: &[u8]) -> Self {
        match self {
            Self::Bytes(_) => Self::Bytes(Cow::Owned(sub.to_vec())),
            _ => match std::str::from_utf8(sub) {
                Ok(value) => Self::String(Cow::Owned(value.to_string())),
                Err(_) => Self::String(Cow::Owned(String::from_utf8_lossy(sub).into_owned())),
            },
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
            (left, right) => match (
                left.as_number().ok().flatten(),
                right.as_number().ok().flatten(),
            ) {
                (Some(left), Some(right)) => left.add(right).map(Self::Number).map_err(jsonb_error),
                _ => match (left.materialize_current()?, right.materialize_current()?) {
                    (Self::String(left), Self::String(right)) => Ok(Self::String(Cow::Owned(
                        format!("{}{}", left.as_ref(), right.as_ref()),
                    ))),
                    (Self::Bytes(left), Self::Bytes(right)) => {
                        let mut bytes = left.into_owned();
                        bytes.extend_from_slice(&right);
                        Ok(Self::Bytes(Cow::Owned(bytes)))
                    }
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
        match (
            self.as_number().ok().flatten(),
            rhs.as_number().ok().flatten(),
        ) {
            (Some(left), Some(right)) => left.sub(right).map(Self::Number).map_err(jsonb_error),
            _ => match (self.materialize_current()?, rhs.materialize_current()?) {
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
        match (
            self.as_number().ok().flatten(),
            rhs.as_number().ok().flatten(),
        ) {
            (Some(left), Some(right)) => left.mul(right).map(Self::Number).map_err(jsonb_error),
            _ => match (self.materialize_current()?, rhs.materialize_current()?) {
                (Self::String(value), Self::Number(count))
                | (Self::Number(count), Self::String(value)) => repeat_string(value, count),
                (Self::Bytes(value), Self::Number(count))
                | (Self::Number(count), Self::Bytes(value)) => repeat_bytes(value, count),
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
        match (
            self.as_number().ok().flatten(),
            rhs.as_number().ok().flatten(),
        ) {
            (Some(left), Some(right)) => Ok(Self::Number(Number::Float64(
                left.as_f64() / right.as_f64(),
            ))),
            _ => match (self.materialize_current()?, rhs.materialize_current()?) {
                (Self::String(left), Self::String(right)) => Ok(split_string(left, right)),
                (Self::Bytes(left), Self::Bytes(right)) => Ok(split_bytes(left, right)),
                (left, right) => Err(jaq_core::Error::math(left, jaq_core::ops::Math::Div, right)),
            },
        }
    }
}

impl<'a> Rem for QueryValue<'a> {
    type Output = ValR<Self>;

    fn rem(self, rhs: Self) -> Self::Output {
        match (
            self.as_number().ok().flatten(),
            rhs.as_number().ok().flatten(),
        ) {
            (Some(left), Some(right)) => match (integer_number(&left), integer_number(&right)) {
                (Some(_), Some(0)) => Err(jaq_core::Error::math(
                    Self::Number(left),
                    jaq_core::ops::Math::Rem,
                    Self::Number(right),
                )),
                (Some(_), Some(_)) => left.rem(right).map(Self::Number).map_err(jsonb_error),
                _ => Ok(Self::Number(Number::Float64(
                    left.as_f64() % right.as_f64(),
                ))),
            },
            _ => {
                let left = self.materialize_current()?;
                let right = rhs.materialize_current()?;
                Err(jaq_core::Error::math(left, jaq_core::ops::Math::Rem, right))
            }
        }
    }
}

impl<'a> Neg for QueryValue<'a> {
    type Output = ValR<Self>;

    fn neg(self) -> Self::Output {
        match self.as_number().ok().flatten() {
            Some(number) => number.neg().map(Self::Number).map_err(jsonb_error),
            None => Err(jaq_core::Error::typ(self.materialize_current()?, "number")),
        }
    }
}

#[cfg(test)]
mod tests {
    use jaq_core::load::Arena;
    use jaq_core::load::File;
    use jaq_core::load::Loader;
    use jaq_core::unwrap_valr;
    use jaq_core::Compiler;
    use jaq_core::Ctx;
    use jaq_core::ValT;
    use jaq_core::Vars;

    use crate::core::QueryValue;
    use crate::jaq::defs;
    use crate::jaq::funs;
    use crate::jaq::JsonbData;
    use crate::OwnedJsonb;

    fn query_value(json: &str) -> QueryValue<'static> {
        QueryValue::from_owned(json.parse::<OwnedJsonb>().unwrap())
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

        let input = QueryValue::from_owned(input.parse::<OwnedJsonb>().unwrap());
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
