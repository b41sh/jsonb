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
use std::rc::Rc;

use jaq_core::val::Range;
use jaq_core::val::ValR;

use crate::core::QueryValue;
use crate::Error;

pub(super) fn str_error<'a>(message: impl ToString) -> jaq_core::Error<QueryValue<'a>> {
    jaq_core::Error::str(message)
}

pub(super) fn jsonb_error<'a>(error: Error) -> jaq_core::Error<QueryValue<'a>> {
    str_error(error)
}

pub(super) fn abs_index(index: isize, len: usize) -> Option<usize> {
    if index >= 0 {
        usize::try_from(index).ok().filter(|index| *index < len)
    } else {
        len.checked_sub(index.unsigned_abs())
    }
}

pub(super) fn range_bound<'a>(
    bound: Option<&QueryValue<'a>>,
    len: usize,
    default: usize,
) -> ValR<usize, QueryValue<'static>> {
    match bound {
        None | Some(QueryValue::Null) => Ok(default),
        Some(value) => {
            let index = value.as_isize().ok().flatten().ok_or_else(|| {
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

pub(super) fn string_range<'a>(
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

impl<'a> QueryValue<'a> {
    pub(super) fn raw_value(&'a self) -> Option<crate::RawJsonb<'a>> {
        match self {
            QueryValue::Raw(raw) => Some(*raw),
            QueryValue::Owned(owned) => Some(owned.as_raw()),
            _ => None,
        }
    }

    pub(super) fn as_str_owned(&self) -> Option<String> {
        self.as_key_string().ok().flatten()
    }

    pub(super) fn materialize(self) -> ValR<QueryValue<'static>, QueryValue<'static>> {
        match self {
            QueryValue::Raw(raw) => raw.to_query_value(),
            QueryValue::Owned(owned) => owned.as_raw().to_query_value(),
            value => Ok(value.into_owned_static()),
        }
    }

    pub(super) fn materialize_current(self) -> ValR<QueryValue<'a>> {
        self.materialize().map(QueryValue::from_static)
    }

    pub(super) fn into_array(self) -> ValR<Rc<Vec<QueryValue<'a>>>, QueryValue<'a>> {
        match self.materialize_current()? {
            QueryValue::Array(values) => Ok(values),
            value => Err(jaq_core::Error::typ(value, "array")),
        }
    }

    pub(super) fn into_string_value(self) -> ValR<String, QueryValue<'a>> {
        match self.materialize_current()? {
            QueryValue::String(value) => Ok(value.into_owned()),
            value => Err(jaq_core::Error::typ(value, "string")),
        }
    }
}
