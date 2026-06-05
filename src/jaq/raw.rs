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
use std::rc::Rc;

use jaq_core::val::Range;
use jaq_core::val::ValR;

use crate::core::ArrayIterator;
use crate::core::JsonbItem;
use crate::core::JsonbItemType;
use crate::core::ObjectIterator;
use crate::core::ObjectValueIterator;
use crate::core::QueryValue;
use crate::error::Result as JsonbResult;
use crate::ExtensionValue;
use crate::OwnedJsonb;
use crate::RawJsonb;

use super::access::abs_index;
use super::access::array_subsequence_indices;
use super::access::bytes_range;
use super::access::jsonb_error;
use super::access::range_bound;
use super::access::str_error;
use super::access::string_range;

pub(super) fn item_to_query_value<'a>(item: JsonbItem<'a>) -> JsonbResult<QueryValue<'a>> {
    match item {
        JsonbItem::Null => Ok(QueryValue::Null),
        JsonbItem::Boolean(value) => Ok(QueryValue::Bool(value)),
        JsonbItem::Number(value) => value.as_number().map(QueryValue::Number),
        JsonbItem::String(value) => Ok(QueryValue::String(value)),
        JsonbItem::Raw(raw) => Ok(QueryValue::Raw(raw)),
        JsonbItem::Owned(owned) => Ok(QueryValue::from_owned(owned)),
        JsonbItem::Extension(value) => match value.as_extension_value()? {
            ExtensionValue::Binary(value) => Ok(QueryValue::Bytes(Cow::Borrowed(value))),
            _ => OwnedJsonb::from_item(JsonbItem::Extension(value)).map(QueryValue::from_owned),
        },
    }
}

impl<'a> RawJsonb<'a> {
    pub(super) fn raw_str_bytes(self) -> Option<&'a [u8]> {
        match JsonbItem::from_raw_jsonb(self).ok()? {
            JsonbItem::String(value) => match value {
                Cow::Borrowed(value) => Some(value.as_bytes()),
                Cow::Owned(_) => None,
            },
            _ => None,
        }
    }

    pub(super) fn raw_binary_bytes(self) -> Option<&'a [u8]> {
        match JsonbItem::from_raw_jsonb(self).ok()? {
            JsonbItem::Extension(value) => match value.as_extension_value().ok()? {
                ExtensionValue::Binary(value) => Some(value),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn raw_bytes(self) -> Option<&'a [u8]> {
        self.raw_binary_bytes().or_else(|| self.raw_str_bytes())
    }

    pub(super) fn raw_array_values(self) -> ValR<Vec<QueryValue<'static>>, QueryValue<'static>> {
        let iter = ArrayIterator::new(self)
            .map_err(jsonb_error)?
            .ok_or_else(|| str_error("cannot use value as array"))?;
        iter.map(|item| {
            item.map_err(jsonb_error)
                .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                .map(QueryValue::into_owned_static)
        })
        .collect()
    }

    pub(super) fn raw_object_entries(
        self,
    ) -> ValR<Vec<(String, QueryValue<'static>)>, QueryValue<'static>> {
        let iter = ObjectIterator::new(self)
            .map_err(jsonb_error)?
            .ok_or_else(|| str_error("cannot use value as object"))?;
        iter.map(|item| {
            item.map_err(jsonb_error).and_then(|(key, value)| {
                item_to_query_value(value)
                    .map_err(jsonb_error)
                    .map(|value| (key.to_string(), value.into_owned_static()))
            })
        })
        .collect()
    }

    pub(super) fn raw_values(self) -> ValR<Vec<QueryValue<'static>>, QueryValue<'static>> {
        match self.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Array(_) => {
                let iter = ArrayIterator::new(self)
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
                let iter = ObjectValueIterator::new(self)
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
                QueryValue::from_owned(self.to_owned()),
                "iterable (array or object)",
            )),
        }
    }

    pub(super) fn raw_key_values(
        self,
    ) -> ValR<Vec<(QueryValue<'static>, QueryValue<'static>)>, QueryValue<'static>> {
        match self.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Array(_) => self.raw_values().map(|values| {
                values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (QueryValue::from(index), value))
                    .collect()
            }),
            JsonbItemType::Object(_) => {
                let iter = ObjectIterator::new(self)
                    .map_err(jsonb_error)?
                    .ok_or_else(|| str_error("cannot use value as iterable (array or object)"))?;
                iter.map(|item| {
                    item.map_err(jsonb_error).and_then(|(key, value)| {
                        item_to_query_value(value)
                            .map_err(jsonb_error)
                            .map(|value| {
                                (QueryValue::from(key.to_string()), value.into_owned_static())
                            })
                    })
                })
                .collect()
            }
            _ => Err(jaq_core::Error::typ(
                QueryValue::from_owned(self.to_owned()),
                "iterable (array or object)",
            )),
        }
    }

    pub(super) fn raw_index<'b>(
        self,
        index: &QueryValue<'b>,
    ) -> ValR<QueryValue<'static>, QueryValue<'static>> {
        self.raw_index_borrowed(index)
            .map(QueryValue::into_owned_static)
            .map_err(|error| str_error(error.to_string()))
    }

    pub(super) fn raw_index_borrowed<'b>(
        self,
        index: &QueryValue<'b>,
    ) -> ValR<QueryValue<'a>, QueryValue<'a>> {
        match self.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Array(len) => {
                if let Some(range) = index.as_range_object_owned()? {
                    return self
                        .raw_range(range.start.as_ref()..range.end.as_ref())
                        .map(QueryValue::from_static);
                }

                if let Some(needle) = index.as_array_values_owned()? {
                    let values = self.raw_array_values()?;
                    return Ok(QueryValue::from_static(array_subsequence_indices(
                        &values, &needle,
                    )));
                }

                let Some(index_value) = index.as_isize().ok().flatten() else {
                    return Err(jaq_core::Error::index(
                        QueryValue::from_owned(self.to_owned()),
                        QueryValue::from_static(index.clone().into_owned_static()),
                    ));
                };
                let Some(index) = abs_index(index_value, len) else {
                    return Ok(QueryValue::Null);
                };
                let mut iter = ArrayIterator::new(self)
                    .map_err(jsonb_error)?
                    .ok_or_else(|| str_error("cannot use value as array"))?;
                match iter.nth(index) {
                    Some(item) => item
                        .map_err(jsonb_error)
                        .and_then(|item| item_to_query_value(item).map_err(jsonb_error)),
                    None => Ok(QueryValue::Null),
                }
            }
            JsonbItemType::Object(_) => {
                let Some(key) = index.as_key_string().ok().flatten() else {
                    return Ok(QueryValue::Null);
                };
                let key = Cow::Owned(key);
                if let Some(value) = self
                    .get_object_value_by_key_name(&key, |left, right| left == right)
                    .map_err(jsonb_error)?
                {
                    return item_to_query_value(value).map_err(jsonb_error);
                }
                Ok(QueryValue::Null)
            }
            JsonbItemType::String => {
                if let Some(range) = index.as_range_object_owned()? {
                    return self
                        .raw_range(range.start.as_ref()..range.end.as_ref())
                        .map(QueryValue::from_static);
                }
                Err(jaq_core::Error::index(
                    QueryValue::from_owned(self.to_owned()),
                    QueryValue::from_static(index.clone().into_owned_static()),
                ))
            }
            JsonbItemType::Extension => {
                if self.raw_binary_bytes().is_some() {
                    if let Some(range) = index.as_range_object_owned()? {
                        return self
                            .raw_range(range.start.as_ref()..range.end.as_ref())
                            .map(QueryValue::from_static);
                    }
                }
                Err(jaq_core::Error::index(
                    QueryValue::from_owned(self.to_owned()),
                    QueryValue::from_static(index.clone().into_owned_static()),
                ))
            }
            JsonbItemType::Null => Ok(QueryValue::Null),
            _ => Err(jaq_core::Error::index(
                QueryValue::from_owned(self.to_owned()),
                QueryValue::from_static(index.clone().into_owned_static()),
            )),
        }
    }

    pub(super) fn raw_range<'b>(
        self,
        range: Range<&QueryValue<'b>>,
    ) -> ValR<QueryValue<'static>, QueryValue<'static>> {
        match self.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Array(len) => {
                let start = range_bound(range.start, len, 0)?;
                let end = range_bound(range.end, len, len)?;
                let take = end.saturating_sub(start);
                let iter = ArrayIterator::new(self)
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
                let value = self
                    .as_str()
                    .map_err(jsonb_error)?
                    .ok_or_else(|| str_error("cannot use value as string"))?;
                string_range(value, range)
            }
            JsonbItemType::Extension => {
                let value = self
                    .as_binary()
                    .map_err(jsonb_error)?
                    .ok_or_else(|| str_error("cannot use value as string"))?;
                bytes_range(Cow::Owned(value), range)
            }
            _ => Err(jaq_core::Error::typ(
                QueryValue::from_owned(self.to_owned()),
                "rangeable (array or string)",
            )),
        }
    }

    pub(super) fn to_query_value(self) -> ValR<QueryValue<'static>, QueryValue<'static>> {
        match self.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Array(_) => self
                .raw_values()
                .map(|values| QueryValue::Array(Rc::new(values))),
            JsonbItemType::Object(_) => self.raw_key_values().map(|values| {
                QueryValue::Object(Rc::new(values.into_iter().collect::<BTreeMap<_, _>>()))
            }),
            _ => JsonbItem::from_raw_jsonb(self)
                .map_err(jsonb_error)
                .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                .map(QueryValue::into_owned_static),
        }
    }
}
