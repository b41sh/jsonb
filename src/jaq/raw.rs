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
use crate::OwnedJsonb;
use crate::RawJsonb;

use super::access::abs_index;
use super::access::jsonb_error;
use super::access::range_bound;
use super::access::str_error;
use super::access::string_range;

pub(super) fn item_to_query_value<'a>(item: JsonbItem<'a>) -> JsonbResult<QueryValue<'a>> {
    match item {
        JsonbItem::Null => Ok(QueryValue::Null),
        JsonbItem::Boolean(value) => Ok(QueryValue::Bool(value)),
        JsonbItem::Number(value) => value.as_number().map(QueryValue::Number),
        JsonbItem::String(value) => Ok(QueryValue::String(Cow::Owned(value.into_owned()))),
        JsonbItem::Raw(raw) => Ok(QueryValue::from_owned(raw.to_owned())),
        JsonbItem::Owned(owned) => Ok(QueryValue::from_owned(owned)),
        JsonbItem::Extension(value) => {
            OwnedJsonb::from_item(JsonbItem::Extension(value)).map(QueryValue::from_owned)
        }
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
        match self.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Array(len) => {
                let Some(index_value) = index.as_isize().ok().flatten() else {
                    return Err(jaq_core::Error::index(
                        QueryValue::from_owned(self.to_owned()),
                        index.clone().into_owned_static(),
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
                        .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
                        .map(QueryValue::into_owned_static),
                    None => Ok(QueryValue::Null),
                }
            }
            JsonbItemType::Object(_) => {
                let Some(key) = index.as_key_string().ok().flatten() else {
                    return Ok(QueryValue::Null);
                };
                let iter = ObjectIterator::new(self)
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
                QueryValue::from_owned(self.to_owned()),
                index.clone().into_owned_static(),
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
