use std::borrow::Cow;

use jaq_core::native::{bome, run, unary, v, Filter, Fun};
use jaq_core::{load, DataT, RunPtr, ValR};

use crate::core::QueryValue;
use crate::core::{ArrayIterator, JsonbItem, JsonbItemType, ObjectIterator};
use crate::{parse_owned_jsonb, Number, OwnedJsonb, RawJsonb};

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
        ("fromjson", v(0), |cv| bome(fromjson(cv.1))),
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

fn jsonb_error<'a>(error: crate::Error) -> jaq_core::Error<QueryValue<'a>> {
    str_error(error)
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
            _ => Ok(false),
        };
    }

    match value {
        QueryValue::Array(values) => Ok(as_index(key)
            .and_then(|index| abs_index(index, values.len()))
            .is_some()),
        QueryValue::Object(values) => Ok(values.contains_key(key)),
        _ => Ok(false),
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

    let values = array_values(value)?;
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

fn fromjson(value: QueryValue<'_>) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    let Some(value) = as_str_owned(&value) else {
        return Err(jaq_core::Error::typ(value.into_owned_static(), "string"));
    };
    parse_owned_jsonb(value.as_bytes())
        .map(QueryValue::Owned)
        .map_err(jsonb_error)
}

fn tobytes(value: QueryValue<'_>) -> ValR<QueryValue<'static>, QueryValue<'static>> {
    fn byte(value: &QueryValue<'_>) -> Option<u8> {
        as_number(value)
            .and_then(|number| number.as_u64())
            .and_then(|number| u8::try_from(number).ok())
    }

    match value {
        QueryValue::String(value) => Ok(QueryValue::String(Cow::Owned(value.into_owned()))),
        value if as_str_owned(&value).is_some() => Ok(QueryValue::String(Cow::Owned(
            as_str_owned(&value).unwrap(),
        ))),
        value if byte(&value).is_some() => {
            let byte = byte(&value).unwrap();
            Ok(QueryValue::String(Cow::Owned(
                String::from_utf8_lossy(&[byte]).into_owned(),
            )))
        }
        value => {
            let values = array_values(&value)?;
            let mut bytes = Vec::with_capacity(values.len());
            for value in values {
                let Some(byte) = byte(&value) else {
                    return Err(str_error(format!("cannot convert {value} to bytes")));
                };
                bytes.push(byte);
            }
            Ok(QueryValue::String(Cow::Owned(
                String::from_utf8_lossy(&bytes).into_owned(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{unwrap_valr, Compiler, Ctx, Vars};

    use super::{defs, funs};
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
