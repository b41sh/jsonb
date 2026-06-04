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

use jaq_core::load;
use jaq_core::native::bome;
use jaq_core::native::run;
use jaq_core::native::unary;
use jaq_core::native::v;
use jaq_core::native::Filter;
use jaq_core::native::Fun;
use jaq_core::DataT;
use jaq_core::Exn;
use jaq_core::RunPtr;
use jaq_core::ValR;
use jaq_core::ValXs;

use crate::core::ArrayIterator;
use crate::core::JsonbItem;
use crate::core::JsonbItemType;
use crate::core::ObjectIterator;
use crate::core::QueryValue;
use crate::error::Result as JsonbResult;
use crate::parse_owned_jsonb;
use crate::Error;
use crate::Number;
use crate::OwnedJsonb;
use crate::RawJsonb;

const DEFS: &str = r#"
def totype(p; e): if p then . else fromjson | if p then . else e end end;
def tonumber : totype(isnumber ; error("cannot parse as number" ));
def toboolean: totype(isboolean; error("cannot parse as boolean"));
def transpose: [range([.[] | length] | max) as $i | [.[][$i]]];
def in(xs)    : . as $x | xs | has     ($x);
def inside(xs): . as $x | xs | contains($x);
def  index($i): indices($i)[ 0];
def rindex($i): indices($i)[-1];
def @json: tojson;
"#;

/// JSONB-specific jaq definitions.
pub fn defs() -> impl Iterator<Item = load::parse::Def<&'static str>> {
    load::parse(DEFS, |p| p.defs()).unwrap().into_iter()
}

/// JSONB-specific jaq native functions.
pub fn funs<D>() -> impl Iterator<Item = Fun<D>>
where
    D: for<'a> DataT<V<'a> = QueryValue<'a>>,
{
    base().into_vec().into_iter().map(run)
}

fn base<D>() -> Box<[Filter<RunPtr<D>>]>
where
    D: for<'a> DataT<V<'a> = QueryValue<'a>>,
{
    Box::new([
        ("fromjson", v(0), |cv| fromjson(cv.1)),
        ("tojson", v(0), |cv| bome(Ok(tojson(cv.1)))),
        ("tobytes", v(0), |cv| bome(tobytes(cv.1))),
        ("length", v(0), |cv| bome(length(&cv.1))),
        ("contains", v(1), |cv| {
            unary(cv, |value, needle| {
                Ok(QueryValue::from(contains(&value, &needle)))
            })
        }),
        ("has", v(1), |cv| {
            unary(cv, |value, key| Ok(QueryValue::from(has(&value, &key)?)))
        }),
        ("indices", v(1), |cv| {
            unary(cv, |value, needle| {
                indices(&value, &needle).map(|values| values.into_iter().collect())
            })
        }),
        ("bsearch", v(1), |cv| {
            unary(cv, |value, needle| {
                let values = array_values(&value)?;
                let index = values.binary_search(&needle).map_or_else(
                    |index| -1 - isize::try_from(index).unwrap_or(isize::MAX),
                    |index| isize::try_from(index).unwrap_or(isize::MAX),
                );
                Ok(QueryValue::from(index))
            })
        }),
    ])
}

fn str_error<'a>(message: impl ToString) -> jaq_core::Error<QueryValue<'a>> {
    jaq_core::Error::str(message)
}

fn jsonb_error<'a>(error: Error) -> jaq_core::Error<QueryValue<'a>> {
    str_error(error)
}

fn parse_fail<'a>(
    input: impl std::fmt::Display,
    format: &str,
    error: impl std::fmt::Display,
) -> jaq_core::Error<QueryValue<'a>> {
    jaq_core::Error::str(format!("cannot parse {input} as {format}: {error}"))
}

fn item_to_query_value<'a>(item: JsonbItem<'a>) -> JsonbResult<QueryValue<'a>> {
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

fn raw_array_values(raw: RawJsonb<'_>) -> ValR<Vec<QueryValue<'static>>, QueryValue<'static>> {
    let iter = ArrayIterator::new(raw)
        .map_err(jsonb_error)?
        .ok_or_else(|| str_error("cannot use value as array"))?;
    iter.map(|item| {
        item.map_err(jsonb_error)
            .and_then(|item| item_to_query_value(item).map_err(jsonb_error))
            .map(QueryValue::into_owned_static)
    })
    .collect()
}

fn raw_object_entries(
    raw: RawJsonb<'_>,
) -> ValR<Vec<(String, QueryValue<'static>)>, QueryValue<'static>> {
    let iter = ObjectIterator::new(raw)
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

fn raw_value<'a>(value: &'a QueryValue<'_>) -> Option<RawJsonb<'a>> {
    match value {
        QueryValue::Raw(raw) => Some(*raw),
        QueryValue::Owned(owned) => Some(owned.as_raw()),
        _ => None,
    }
}

fn as_number(value: &QueryValue<'_>) -> Option<Number> {
    value.as_number().ok().flatten()
}

fn as_key_string(value: &QueryValue<'_>) -> Option<String> {
    value
        .as_string()
        .ok()
        .flatten()
        .map(|value| value.into_owned())
}

fn as_str_owned(value: &QueryValue<'_>) -> Option<String> {
    as_key_string(value)
}

fn as_index(value: &QueryValue<'_>) -> Option<isize> {
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

fn number_abs(number: Number) -> Number {
    match number {
        Number::Int64(value) => value
            .checked_abs()
            .map(Number::Int64)
            .unwrap_or(Number::Float64((value as f64).abs())),
        Number::UInt64(_) => number,
        Number::Float64(value) => Number::Float64(value.abs()),
        value => Number::Float64(value.as_f64().abs()),
    }
}

fn length(value: &QueryValue<'_>) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    if let Some(raw) = raw_value(value) {
        return match raw.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Null => Ok(QueryValue::from(0usize)),
            JsonbItemType::Number => raw
                .as_number()
                .map_err(jsonb_error)?
                .map(number_abs)
                .map(QueryValue::Number)
                .ok_or_else(|| str_error("number expected")),
            JsonbItemType::String => raw
                .as_str()
                .map_err(jsonb_error)?
                .map(|value| QueryValue::from(value.chars().count()))
                .ok_or_else(|| str_error("string expected")),
            JsonbItemType::Array(len) | JsonbItemType::Object(len) => Ok(QueryValue::from(len)),
            JsonbItemType::Boolean | JsonbItemType::Extension => {
                Err(str_error(format!("{value} has no length")))
            }
        };
    }

    match value {
        QueryValue::Null => Ok(QueryValue::from(0usize)),
        QueryValue::Number(number) => Ok(QueryValue::Number(number_abs(number.clone()))),
        QueryValue::String(value) => Ok(QueryValue::from(value.chars().count())),
        QueryValue::Array(values) => Ok(QueryValue::from(values.len())),
        QueryValue::Object(values) => Ok(QueryValue::from(values.len())),
        QueryValue::Bool(_) => Err(str_error(format!("{value} has no length"))),
        QueryValue::Raw(_) | QueryValue::Owned(_) => unreachable!(),
    }
}

fn has(value: &QueryValue<'_>, key: &QueryValue<'_>) -> ValR<bool, QueryValue<'static>> {
    if let Some(raw) = raw_value(value) {
        return match raw.jsonb_item_type().map_err(jsonb_error)? {
            JsonbItemType::Null => Ok(false),
            JsonbItemType::Array(len) => {
                Ok(as_index(key).and_then(|i| abs_index(i, len)).is_some())
            }
            JsonbItemType::Object(_) => {
                let Some(key) = as_key_string(key) else {
                    return Ok(false);
                };
                Ok(raw_object_entries(raw)?
                    .into_iter()
                    .any(|(entry, _)| entry == key))
            }
            _ => Err(jaq_core::Error::index(
                value.clone().into_owned_static(),
                key.clone().into_owned_static(),
            )),
        };
    }

    match value {
        QueryValue::Null => Ok(false),
        QueryValue::Array(values) => Ok(as_index(key)
            .and_then(|index| abs_index(index, values.len()))
            .is_some()),
        QueryValue::Object(values) => Ok(values.contains_key(key)),
        value => Err(jaq_core::Error::index(
            value.clone().into_owned_static(),
            key.clone().into_owned_static(),
        )),
    }
}

fn array_values(value: &QueryValue<'_>) -> ValR<Vec<QueryValue<'static>>, QueryValue<'static>> {
    if let Some(raw) = raw_value(value) {
        return raw_array_values(raw);
    }
    match value {
        QueryValue::Array(values) => Ok(values
            .iter()
            .cloned()
            .map(QueryValue::into_owned_static)
            .collect()),
        value => Err(jaq_core::Error::typ(
            value.clone().into_owned_static(),
            "array",
        )),
    }
}

fn object_entries(
    value: &QueryValue<'_>,
) -> ValR<Vec<(String, QueryValue<'static>)>, QueryValue<'static>> {
    if let Some(raw) = raw_value(value) {
        return raw_object_entries(raw);
    }
    match value {
        QueryValue::Object(values) => values
            .iter()
            .map(|(key, value)| {
                let Some(key) = as_key_string(key) else {
                    return Err(str_error("object key is not a string"));
                };
                Ok((key, value.clone().into_owned_static()))
            })
            .collect(),
        value => Err(jaq_core::Error::typ(
            value.clone().into_owned_static(),
            "object",
        )),
    }
}

fn contains(value: &QueryValue<'_>, needle: &QueryValue<'_>) -> bool {
    if let (Some(value), Some(needle)) = (as_str_owned(value), as_str_owned(needle)) {
        return value.contains(&needle);
    }

    if let (Ok(values), Ok(needles)) = (array_values(value), array_values(needle)) {
        return needles
            .iter()
            .all(|needle| values.iter().any(|value| contains(value, needle)));
    }

    if let (Ok(values), Ok(needles)) = (object_entries(value), object_entries(needle)) {
        return needles.into_iter().all(|(needle_key, needle_value)| {
            values
                .iter()
                .find(|(key, _)| key == &needle_key)
                .is_some_and(|(_, value)| contains(value, &needle_value))
        });
    }

    value.clone().into_owned_static() == needle.clone().into_owned_static()
}

fn indices(
    value: &QueryValue<'_>,
    needle: &QueryValue<'_>,
) -> ValR<Vec<QueryValue<'static>>, QueryValue<'static>> {
    if let (Some(value), Some(needle)) = (as_str_owned(value), as_str_owned(needle)) {
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for (char_index, (byte_index, _)) in value.char_indices().enumerate() {
            if value[byte_index..].starts_with(&needle) {
                result.push(QueryValue::from(char_index));
            }
        }
        return Ok(result);
    }

    let values = match array_values(value) {
        Ok(values) => values,
        Err(_) => {
            return Err(jaq_core::Error::index(
                value.clone().into_owned_static(),
                needle.clone().into_owned_static(),
            ))
        }
    };
    let needles = match array_values(needle) {
        Ok(needles) => needles,
        Err(_) => {
            return Ok(values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    (value == &needle.clone().into_owned_static()).then(|| QueryValue::from(index))
                })
                .collect());
        }
    };

    if needles.is_empty() {
        return Ok(Vec::new());
    }

    Ok(values
        .windows(needles.len())
        .enumerate()
        .filter_map(|(index, window)| (window == needles).then(|| QueryValue::from(index)))
        .collect())
}

fn tojson(value: QueryValue<'_>) -> QueryValue<'static> {
    QueryValue::String(Cow::Owned(value.to_string()))
}

fn json_value_segments(input: &str) -> ValR<Vec<&str>, QueryValue<'static>> {
    fn skip_ws(bytes: &[u8], index: &mut usize) {
        while bytes
            .get(*index)
            .is_some_and(|value| value.is_ascii_whitespace())
        {
            *index += 1;
        }
    }

    fn scan_string(bytes: &[u8], index: &mut usize, quote: u8) -> ValR<(), QueryValue<'static>> {
        *index += 1;
        while let Some(value) = bytes.get(*index) {
            match *value {
                b'\\' => *index += 2,
                value if value == quote => {
                    *index += 1;
                    return Ok(());
                }
                _ => *index += 1,
            }
        }
        Err(str_error("unterminated string"))
    }

    fn scan_container(bytes: &[u8], index: &mut usize) -> ValR<(), QueryValue<'static>> {
        let mut stack = Vec::new();
        loop {
            let Some(value) = bytes.get(*index) else {
                return Err(str_error("unterminated JSON container"));
            };
            match *value {
                b'"' | b'\'' => scan_string(bytes, index, *value)?,
                b'[' | b'{' => {
                    stack.push(if *value == b'[' { b']' } else { b'}' });
                    *index += 1;
                }
                b']' | b'}' => {
                    let Some(expected) = stack.pop() else {
                        return Err(str_error("unexpected JSON container close"));
                    };
                    if *value != expected {
                        return Err(str_error("mismatched JSON container close"));
                    }
                    *index += 1;
                    if stack.is_empty() {
                        return Ok(());
                    }
                }
                _ => *index += 1,
            }
        }
    }

    let bytes = input.as_bytes();
    let mut index = 0;
    let mut segments = Vec::new();
    loop {
        skip_ws(bytes, &mut index);
        if index >= bytes.len() {
            break;
        }
        let start = index;
        match bytes[index] {
            b'"' | b'\'' => {
                let quote = bytes[index];
                scan_string(bytes, &mut index, quote)?;
            }
            b'[' | b'{' => scan_container(bytes, &mut index)?,
            _ => {
                while bytes
                    .get(index)
                    .is_some_and(|value| !value.is_ascii_whitespace())
                {
                    index += 1;
                }
            }
        }
        segments.push(&input[start..index]);
    }
    Ok(segments)
}

fn fromjson<'a>(value: QueryValue<'a>) -> ValXs<'a, QueryValue<'a>> {
    let input_display = value.to_string();
    let Some(input) = as_str_owned(&value) else {
        return Box::new(std::iter::once(Err(Exn::from(jaq_core::Error::typ(
            value, "string",
        )))));
    };

    let results = match json_value_segments(&input) {
        Ok(segments) => segments
            .into_iter()
            .map(|segment| {
                parse_owned_jsonb(segment.as_bytes())
                    .map(QueryValue::Owned)
                    .map(QueryValue::from_static)
                    .map_err(|error| Exn::from(parse_fail(&input_display, "JSON", error)))
            })
            .collect::<Vec<_>>(),
        Err(error) => vec![Err(Exn::from(error))],
    };
    Box::new(results.into_iter())
}

fn to_bytes(value: &QueryValue<'_>) -> std::result::Result<Vec<u8>, QueryValue<'static>> {
    fn byte(value: &QueryValue<'_>) -> Option<u8> {
        as_number(value)
            .and_then(|number| number.as_u64())
            .and_then(|number| u8::try_from(number).ok())
    }

    if let Some(byte) = byte(value) {
        return Ok(vec![byte]);
    }

    if let Some(value) = as_str_owned(value) {
        return Ok(value.into_bytes());
    }

    match value {
        QueryValue::Array(values) => {
            let mut bytes = Vec::new();
            for value in values.iter() {
                bytes.extend(to_bytes(value)?);
            }
            Ok(bytes)
        }
        value if raw_value(value).is_some() => {
            let values = array_values(value).map_err(|_| value.clone().into_owned_static())?;
            let mut bytes = Vec::new();
            for value in values {
                bytes.extend(to_bytes(&value)?);
            }
            Ok(bytes)
        }
        value => Err(value.clone().into_owned_static()),
    }
}

fn tobytes(value: QueryValue<'_>) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    to_bytes(&value)
        .map(|bytes| QueryValue::String(Cow::Owned(String::from_utf8_lossy(&bytes).into_owned())))
        .map_err(|value| str_error(format!("cannot convert {value} to bytes")))
}

#[cfg(test)]
mod tests {
    use jaq_core::load::Arena;
    use jaq_core::load::File;
    use jaq_core::load::Loader;
    use jaq_core::unwrap_valr;
    use jaq_core::Compiler;
    use jaq_core::Ctx;
    use jaq_core::Vars;

    use super::defs;
    use super::funs;
    use crate::core::QueryValue;
    use crate::jaq::JsonbData;
    use crate::OwnedJsonb;

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
    fn native_jsonb_functions_run_without_jaq_json() {
        assert_eq!(run_filter("length", r#"{"a":1,"b":2}"#), ["2"]);
        assert_eq!(run_filter(r#"has("a")"#, r#"{"a":null}"#), ["true"]);
        assert_eq!(
            run_filter(r#"contains({"a":1})"#, r#"{"a":1,"b":2}"#),
            ["true"]
        );
        assert_eq!(run_filter(r#""{\"a\":1}" | fromjson | .a"#, "null"), ["1"]);
        assert_eq!(run_filter("tojson", r#"{"a":1}"#), [r#""{\"a\":1}""#]);
    }

    #[test]
    fn native_jsonb_functions_match_jaq_json_edges() {
        assert_eq!(run_filter("[1, 3] | bsearch(0)", "null"), ["-1"]);
        assert_eq!(run_filter("[1, 3] | bsearch(2)", "null"), ["-2"]);
        assert_eq!(run_filter("[1, 3] | [bsearch(1, 3)]", "null"), ["[0,1]"]);

        assert_eq!(
            run_filter(
                r#""Infinity +Infinity -Infinity" | [fromjson | tostring]"#,
                "null"
            ),
            [r#"["Infinity","Infinity","-Infinity"]"#]
        );
        assert_eq!(run_filter(r#"" 1" | fromjson"#, "null"), ["1"]);
        assert_eq!(run_filter(r#""+1" | fromjson"#, "null"), ["1"]);
        assert_eq!(run_filter(r#""-1" | fromjson"#, "null"), ["-1"]);

        assert_eq!(
            run_filter(r#""a,b, cd, efg" | indices(", ")"#, "null"),
            ["[3,7]"]
        );
        assert_eq!(
            run_filter("[0, 1, 2, 1, 3, 1, 4] | indices(1)", "null"),
            ["[1,3,5]"]
        );
        assert_eq!(
            run_filter(
                "[0, 1, 2, 3, 1, 4, 2, 5, 1, 2, 6, 7] | indices([1, 2])",
                "null"
            ),
            ["[1,8]"]
        );
        assert_eq!(run_filter(r#""🇬🇧🇬🇧" | indices("🇬🇧")"#, "null"), ["[0,2]"]);

        assert_eq!(run_filter(r#""ƒoo" | length"#, "null"), ["3"]);
        assert_eq!(run_filter(r#""नमस्ते" | length"#, "null"), ["6"]);
        assert_eq!(run_filter("-2.5 | length", "null"), ["2.5"]);
        assert_eq!(
            run_filter(r#"[[65], "B", 67] | tobytes"#, "null"),
            [r#""ABC""#]
        );
    }

    #[test]
    fn constructed_values_encode_to_jsonb() {
        assert_eq!(
            run_filter("[.a, .b, {nested: .c}]", r#"{"a":1,"b":[2],"c":{"d":3}}"#),
            [r#"[1,[2],{"nested":{"d":3}}]"#]
        );
        assert_eq!(
            run_filter("{x: .a, y: [.b, 3], z: null}", r#"{"a":1,"b":2}"#),
            [r#"{"x":1,"y":[2,3],"z":null}"#]
        );
        assert_eq!(
            run_filter(r#"[1, true, null, "x"]"#, "null"),
            [r#"[1,true,null,"x"]"#]
        );
    }
}
