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

//! Shared checks for the jsonb fuzz targets.
//!
//! Every public API must handle arbitrary input without panicking, overflowing
//! the stack, or allocating memory out of proportion to the input. On top of
//! that, each check asserts invariants that hold by design, so a failing
//! assertion is a bug rather than a quirk of malformed input.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

mod model;

use jsonb::jsonpath::parse_json_path;
use jsonb::jsonpath::JsonPath;
use jsonb::keypath::parse_key_paths;
use jsonb::keypath::KeyPath;
use jsonb::keypath::KeyPaths;
use jsonb::keypath::OwnedKeyPaths;
use jsonb::*;
use serde::Deserialize;

/// Valid documents that path and binary-operation checks run against.
const DOCUMENTS: &[&str] = &[
    r#"{"a":{"b":[1,2,{"c":"d"}]},"e":3.25,"f":null,"g":[true,"x",[1,[2]]]}"#,
    r#"[1,"a",null,{"k":[true,2.5]},[[]],{"a":{"a":1}}]"#,
    r#""text""#,
    "-12",
];

/// A type that drives most of serde's `Deserializer` entry points.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Record {
    id: Option<u64>,
    name: String,
    tags: Vec<String>,
    pair: (bool, f64),
    kind: Kind,
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
enum Kind {
    Unit,
    Newtype(i32),
    Tuple(u8, char),
    Struct { x: i16 },
}

fn documents() -> Vec<OwnedJsonb> {
    DOCUMENTS
        .iter()
        .map(|text| text.parse().expect("seed documents are valid JSON"))
        .collect()
}

/// Calls every public read and transform API on a single value.
///
/// Results are discarded: arbitrary bytes may be rejected with any error, but
/// must never panic.
pub fn exercise_unary(raw: RawJsonb<'_>) {
    let keys: BTreeSet<&str> = ["a", "k", ""].into_iter().collect();

    let _ = from_slice(raw.as_ref());
    let _ = raw.to_value().map(|value| value.to_string());
    let _ = raw.to_string();
    let _ = raw.to_pretty_string();
    let _ = serde_json::to_string(&raw);
    let _ = serde_json::to_value(raw);
    let _ = from_raw_jsonb::<serde_json::Value>(&raw);
    let _ = from_raw_jsonb::<String>(&raw);
    let _ = from_raw_jsonb::<char>(&raw);
    let _ = from_raw_jsonb::<i8>(&raw);
    let _ = from_raw_jsonb::<u64>(&raw);
    let _ = from_raw_jsonb::<f32>(&raw);
    let _ = from_raw_jsonb::<Vec<Option<bool>>>(&raw);
    let _ = from_raw_jsonb::<BTreeMap<String, String>>(&raw);
    let _ = from_raw_jsonb::<Record>(&raw);
    let _ = from_raw_jsonb::<Kind>(&raw);
    let _ = jaq_json::Val::try_from(raw).map(|val| val.to_string());

    let _ = raw.jsonb_item_type();
    let _ = raw.type_of();
    let _ = raw.is_null();
    let _ = raw.is_string();
    let _ = raw.as_null();
    let _ = raw.as_bool();
    let _ = raw.to_bool();
    let _ = raw.as_str();
    let _ = raw.to_str();
    let _ = raw.as_number().map(|n| n.map(|n| n.to_string()));
    let _ = raw.as_i64();
    let _ = raw.as_u64();
    let _ = raw.as_f64();
    let _ = raw.to_i64();
    let _ = raw.to_u64();
    let _ = raw.to_f64();
    let _ = raw.as_extension_value().map(|v| v.map(|v| v.to_string()));
    let _ = raw.as_binary();
    let _ = raw.as_date().map(|v| v.map(|v| v.to_string()));
    let _ = raw.as_timestamp().map(|v| v.map(|v| v.to_string()));
    let _ = raw.as_timestamp_tz().map(|v| v.map(|v| v.to_string()));
    let _ = raw.as_interval().map(|v| v.map(|v| v.to_string()));

    let _ = raw.array_length();
    let _ = raw.array_values();
    let _ = raw.array_distinct();
    let _ = raw.array_insert(0, &raw);
    let _ = raw.array_insert(-1, &raw);
    let _ = raw.object_keys();
    let _ = raw.object_each();
    let _ = raw.object_delete(&keys);
    let _ = raw.object_pick(&keys);
    let _ = raw.object_insert("a", &raw, true);
    let _ = raw.object_insert("new", &raw, false);
    let _ = raw.get_by_index(0);
    let _ = raw.get_by_index(usize::MAX);
    let _ = raw.get_by_name("a", false);
    let _ = raw.get_by_name("A", true);
    let _ = raw.delete_by_index(-1);
    let _ = raw.delete_by_index(i32::MIN);
    let _ = raw.delete_by_name("a");
    let _ = raw.exists_any_keys(["a", "k"].into_iter());
    let _ = raw.exists_all_keys(["a", "k"].into_iter());
    let _ = raw.strip_nulls();
    let _ = raw.convert_to_comparable();
    let _ = raw.traverse_check_string(|s| s.is_empty());
    let _ = raw.visit_scalar_key_paths(false, |_| Ok(()));
    let _ = raw.visit_scalar_key_values(true, |_, value| {
        let _ = value.to_string();
        Ok(())
    });
    let _ = raw.partial_cmp(&raw);
    let _ = raw.contains(&raw);
    let _ = raw.concat(&raw);
    let _ = raw.array_intersection(&raw);
    let _ = raw.array_except(&raw);
    let _ = raw.array_overlap(&raw);
    let _ = OwnedJsonb::build_array([raw, raw]).map(|o| o.to_string());
    let _ = OwnedJsonb::build_object([("k", raw)]).map(|o| o.to_string());
}

/// Calls every two-value API on `left` and `right`, in both directions.
pub fn exercise_binary(left: RawJsonb<'_>, right: RawJsonb<'_>) {
    for (a, b) in [(left, right), (right, left)] {
        let _ = a.partial_cmp(&b);
        let _ = a.cmp(&b);
        let _ = a.contains(&b);
        let _ = a.concat(&b).map(|o| o.to_string());
        let _ = a.array_intersection(&b);
        let _ = a.array_except(&b);
        let _ = a.array_overlap(&b);
        let _ = a.array_insert(1, &b);
        let _ = a.object_insert("k", &b, true);
        let _ = a.object_insert("z", &b, false);
        let _ = OwnedJsonb::build_array([a, b]).map(|o| o.to_string());
        let _ = OwnedJsonb::build_object([("a", a), ("b", b)]).map(|o| o.to_string());
    }
}

/// Fuzzes arbitrary bytes interpreted as JSONB.
///
/// Beyond not panicking, a value that decodes must survive a round trip:
/// re-encoding it and decoding again yields an equal value, and that second
/// encoding is stable.
pub fn check_raw_jsonb(data: &[u8]) {
    let raw = RawJsonb::new(data);
    exercise_unary(raw);
    for document in documents() {
        exercise_binary(raw, document.as_raw());
    }

    if let Ok(value) = raw.to_value() {
        let encoded = value.to_vec().expect("a decoded value re-encodes");
        let decoded = RawJsonb::new(&encoded)
            .to_value()
            .expect("re-encoded bytes decode");
        assert_eq!(value, decoded, "round trip changed the value");
        let reencoded = decoded.to_vec().expect("a decoded value re-encodes");
        assert_eq!(encoded, reencoded, "encoding is not stable");
        check_canonical(RawJsonb::new(&encoded));
    }
}

/// Checks invariants of JSONB produced by this crate's own encoder.
fn check_canonical(raw: RawJsonb<'_>) {
    let value = raw.to_value().expect("canonical JSONB decodes");
    assert_eq!(
        from_slice(raw.as_ref()).expect("canonical JSONB decodes with from_slice"),
        value,
        "from_slice and to_value disagree"
    );
    assert_eq!(
        raw.partial_cmp(&raw),
        Some(Ordering::Equal),
        "a value must equal itself"
    );
    check_functions(raw, &value);
}

/// Panics unless a function result matches its model, where `None` stands
/// for an error on one side and a missing value on the other.
fn assert_model(what: impl Fn() -> String, got: Option<OwnedJsonb>, expect: Option<Value<'_>>) {
    match (got, expect) {
        (Some(owned), Some(expect)) => {
            let got = owned.as_raw().to_value().expect("function results decode");
            assert!(got == expect, "{}: got {got}, expected {expect}", what());
        }
        (None, None) => {}
        (got, expect) => panic!(
            "{}: got {:?}, expected {:?}",
            what(),
            got.map(|owned| owned.to_string()),
            expect.map(|value| value.to_string())
        ),
    }
}

/// Names worth probing in `value`: its own keys or string elements, their
/// ASCII case variants, and names that are usually absent.
fn probe_names(value: &Value<'_>) -> Vec<String> {
    let mut names = BTreeSet::new();
    match value {
        Value::Object(object) => names.extend(object.keys().take(4).cloned()),
        Value::Array(items) => names.extend(
            items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .take(4),
        ),
        Value::String(s) => {
            names.insert(s.to_string());
        }
        _ => {}
    }
    let variants: Vec<String> = names
        .iter()
        .flat_map(|name| [name.to_ascii_uppercase(), name.to_ascii_lowercase()])
        .collect();
    names.extend(variants);
    names.extend(["", "a", "missing"].map(String::from));
    names.into_iter().collect()
}

/// Checks `RawJsonb` functions on valid JSONB against the models in
/// [`model`], with arguments taken from the value itself.
fn check_functions(raw: RawJsonb<'_>, value: &Value<'_>) {
    let names = probe_names(value);
    for name in &names {
        for ignore_case in [false, true] {
            assert_model(
                || format!("get_by_name({name:?}, {ignore_case}) on {value}"),
                raw.get_by_name(name, ignore_case).expect("get_by_name"),
                model::get_by_name(value, name, ignore_case),
            );
        }
        assert_model(
            || format!("delete_by_name({name:?}) on {value}"),
            raw.delete_by_name(name).ok(),
            model::delete_by_name(value, name),
        );
        for update in [false, true] {
            assert_model(
                || format!("object_insert({name:?}, self, {update}) on {value}"),
                raw.object_insert(name, &raw, update).ok(),
                model::object_insert(value, name, value, update),
            );
        }
    }

    let first = names.first().map_or("", String::as_str);
    let second = names.get(1).map_or("", String::as_str);
    for keys in [vec![], vec![first], vec![first, second, "missing"]] {
        assert_eq!(
            raw.exists_any_keys(keys.iter().copied())
                .expect("exists_any_keys"),
            model::exists_any_keys(value, &keys),
            "exists_any_keys({keys:?}) on {value}"
        );
        assert_eq!(
            raw.exists_all_keys(keys.iter().copied())
                .expect("exists_all_keys"),
            model::exists_all_keys(value, &keys),
            "exists_all_keys({keys:?}) on {value}"
        );
        let set: BTreeSet<&str> = keys.iter().copied().collect();
        assert_model(
            || format!("object_delete({set:?}) on {value}"),
            raw.object_delete(&set).ok(),
            model::object_delete(value, &set),
        );
        assert_model(
            || format!("object_pick({set:?}) on {value}"),
            raw.object_pick(&set).ok(),
            model::object_pick(value, &set),
        );
    }

    let len = model::array_length(value);
    assert_eq!(
        raw.array_length().expect("array_length"),
        len,
        "array_length of {value}"
    );
    let len = len.unwrap_or(0);
    for index in BTreeSet::from([0, 1, len.wrapping_sub(1), len, len + 1, usize::MAX]) {
        assert_model(
            || format!("get_by_index({index}) on {value}"),
            raw.get_by_index(index).expect("get_by_index"),
            model::get_by_index(value, index),
        );
    }
    let len = i32::try_from(len).unwrap_or(i32::MAX);
    for index in BTreeSet::from([
        0,
        1,
        -1,
        -2,
        len - 1,
        len,
        -len,
        -len - 1,
        i32::MIN,
        i32::MAX,
    ]) {
        assert_model(
            || format!("delete_by_index({index}) on {value}"),
            raw.delete_by_index(index).ok(),
            model::delete_by_index(value, index),
        );
    }

    let got = raw.array_values().expect("array_values");
    let expect = model::array_values(value);
    assert_eq!(
        got.as_ref().map(Vec::len),
        expect.as_ref().map(Vec::len),
        "array_values of {value}"
    );
    for (got, expect) in got.into_iter().flatten().zip(expect.into_iter().flatten()) {
        assert_model(
            || format!("array_values of {value}"),
            Some(got),
            Some(expect),
        );
    }
    assert_model(
        || format!("object_keys of {value}"),
        raw.object_keys().expect("object_keys"),
        model::object_keys(value),
    );
    let got = raw.object_each().expect("object_each");
    let expect = model::object_each(value);
    assert_eq!(
        got.as_ref().map(Vec::len),
        expect.as_ref().map(Vec::len),
        "object_each of {value}"
    );
    for ((got_key, got), (key, expect)) in
        got.into_iter().flatten().zip(expect.into_iter().flatten())
    {
        assert_eq!(got_key, key, "object_each of {value}");
        assert_model(
            || format!("object_each of {value}"),
            Some(got),
            Some(expect),
        );
    }
    assert_model(
        || format!("strip_nulls of {value}"),
        raw.strip_nulls().ok(),
        Some(model::strip_nulls(value)),
    );
    assert!(
        raw.contains(&raw).expect("contains") == model::contains(value, value),
        "contains(self, self) of {value}"
    );

    // Extraction collects exactly what the visitor sees.
    for ignore_array in [false, true] {
        let extracted = raw
            .extract_scalar_key_values(ignore_array)
            .expect("extract_scalar_key_values");
        let mut visited: Vec<(Vec<KeyPath<'_>>, Value<'_>)> = Vec::new();
        raw.visit_scalar_key_values(ignore_array, |paths, value| {
            visited.push((paths.to_vec(), value));
            Ok(())
        })
        .expect("visit_scalar_key_values");
        assert_eq!(
            extracted.len(),
            visited.len(),
            "extract_scalar_key_values of {value}"
        );
        for ((paths, got), (expect_paths, expect)) in extracted.iter().zip(&visited) {
            assert_eq!(
                &paths.paths, expect_paths,
                "extract_scalar_key_values of {value}"
            );
            assert!(got == expect, "extract_scalar_key_values of {value}");
        }
    }
}

/// The JSON text parsers of one syntax mode.
struct ParseMode {
    owned: fn(&[u8]) -> Result<OwnedJsonb, Error>,
    owned_with_buf: fn(&[u8], &mut Vec<u8>) -> Result<(), Error>,
    value: fn(&[u8]) -> Result<Value<'_>, Error>,
}

/// Bytes placed in output buffers before writing, to check that writers
/// append and that a failed write leaves the buffer as it was.
const BUF_PREFIX: &[u8] = b"prefix";

/// Fuzzes arbitrary bytes interpreted as JSON text.
///
/// All encoders (`parse_owned_jsonb`, its `_with_buf` form, and `parse_value`
/// with `to_vec` or `write_to_vec`) must agree byte for byte, `Value` and
/// `RawJsonb` must format the same JSON, and that text must parse back to an
/// equal value. Standard mode must also accept what `serde_json` accepts.
pub fn check_json_text(data: &[u8]) {
    let modes = [
        ParseMode {
            owned: parse_owned_jsonb,
            owned_with_buf: parse_owned_jsonb_with_buf,
            value: parse_value,
        },
        ParseMode {
            owned: parse_owned_jsonb_standard_mode,
            owned_with_buf: parse_owned_jsonb_standard_mode_with_buf,
            value: parse_value_standard_mode,
        },
    ];
    check_against_serde_json(data);
    for mode in modes {
        let owned = (mode.owned)(data);
        let value = (mode.value)(data);

        let mut buf = BUF_PREFIX.to_vec();
        let with_buf = (mode.owned_with_buf)(data, &mut buf);
        let (prefix, written) = buf.split_at(BUF_PREFIX.len());
        assert_eq!(prefix, BUF_PREFIX, "_with_buf changed existing bytes");
        match (&with_buf, &owned) {
            (Ok(()), Ok(owned)) => assert_eq!(written, owned.as_ref(), "_with_buf output differs"),
            (Err(_), Err(_)) => assert!(written.is_empty(), "failed _with_buf left output behind"),
            _ => panic!("_with_buf and owned parsers disagree: {with_buf:?}"),
        }
        let (owned, value) = match (owned, value) {
            (Ok(owned), Ok(value)) => (owned, value),
            (Err(_), Err(_)) => continue,
            (owned, value) => panic!(
                "parsers disagree: owned={:?} value={:?}",
                owned.map(|o| o.to_string()),
                value.map(|v| v.to_string())
            ),
        };
        let encoded = value.to_vec().expect("a parsed value encodes");
        assert_eq!(owned.as_ref(), encoded.as_slice(), "encoders disagree");
        let mut buf = BUF_PREFIX.to_vec();
        value
            .write_to_vec(&mut buf)
            .expect("a parsed value encodes");
        assert_eq!(
            &buf[BUF_PREFIX.len()..],
            encoded.as_slice(),
            "write_to_vec differs"
        );

        let raw = owned.as_raw();
        check_canonical(raw);
        exercise_unary(raw);

        // `Value` writes the same JSON as `RawJsonb`, so it is valid JSON too.
        let value_text = value.to_string();
        assert_eq!(
            value_text,
            raw.to_string(),
            "Value and RawJsonb format differently"
        );
        if let Err(err) = serde_json::from_str::<serde_json::Value>(&value_text) {
            panic!("formatted text {value_text:?} is not JSON: {err}");
        }

        // Formatting and reparsing keeps the value. The text itself may change
        // once, e.g. `Float64(-0.0)` prints as `-0.0`, which parses back as the
        // decimal `0.0`, but after that it must be stable. JSON text has no
        // NaN or infinity, so those print as `null` and only stability holds.
        let reparse = |text: &str| {
            parse_owned_jsonb(text.as_bytes())
                .unwrap_or_else(|err| panic!("formatted text {text:?} does not parse: {err}"))
        };
        let text = raw.to_string();
        let reparsed = reparse(&text);
        if !has_non_finite_float(&value) {
            assert_eq!(
                reparsed.as_raw().partial_cmp(&raw),
                Some(Ordering::Equal),
                "reparsing {text:?} changed the value"
            );
        }
        let stable_text = reparsed.to_string();
        assert_eq!(
            reparse(&stable_text).to_string(),
            stable_text,
            "formatting is not stable"
        );
    }
}

/// Checks that standard mode accepts what `serde_json` accepts, with the same
/// meaning.
///
/// Only this direction is checked: standard mode is knowingly more lenient,
/// accepting raw control characters and lone surrogates that `serde_json`
/// rejects. Duplicate object keys are the one input `serde_json` accepts and
/// standard mode rejects by design.
fn check_against_serde_json(data: &[u8]) {
    let Ok(expect) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    let got = match parse_value_standard_mode(data) {
        Ok(got) => got,
        Err(err) if err.to_string().starts_with("duplicate object attribute") => return,
        Err(err) => panic!("standard mode rejects JSON that serde_json accepts: {err}"),
    };
    assert!(
        same_as_serde_json(&got, &expect),
        "standard mode parses {got}, serde_json parses {expect}"
    );
}

/// Compares a parsed `Value` with a `serde_json::Value` of the same text.
fn same_as_serde_json(got: &Value<'_>, expect: &serde_json::Value) -> bool {
    match (got, expect) {
        (Value::Null, serde_json::Value::Null) => true,
        (Value::Bool(a), serde_json::Value::Bool(b)) => a == b,
        (Value::String(a), serde_json::Value::String(b)) => a == b,
        // Number representations differ by design (decimals, overflow to
        // infinity), so compare with this crate's reading of serde_json's
        // digits, which `arbitrary_precision` keeps as written.
        (Value::Number(a), serde_json::Value::Number(b)) => {
            matches!(parse_value_standard_mode(b.to_string().as_bytes()), Ok(Value::Number(b)) if *a == b)
        }
        (Value::Array(a), serde_json::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_as_serde_json(a, b))
        }
        (Value::Object(a), serde_json::Value::Object(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(key, b)| a.get(key).is_some_and(|a| same_as_serde_json(a, b)))
        }
        _ => false,
    }
}

/// Returns whether `value` contains NaN or an infinity, which JSON text
/// cannot represent.
fn has_non_finite_float(value: &Value<'_>) -> bool {
    match value {
        Value::Number(Number::Float64(v)) => !v.is_finite(),
        Value::Array(items) => items.iter().any(has_non_finite_float),
        Value::Object(object) => object.values().any(has_non_finite_float),
        _ => false,
    }
}

/// Fuzzes arbitrary bytes interpreted as a JSONPath or key-path expression.
///
/// A path that parses must format to text that parses back to the same path,
/// and evaluating it must not panic.
pub fn check_path(data: &[u8]) {
    let documents = documents();
    let json_path = parse_json_path(data);
    if let Ok(path) = &json_path {
        for document in &documents {
            let raw = document.as_raw();
            let _ = raw.select_by_path(path);
            let _ = raw.select_array_by_path(path);
            let _ = raw.select_first_by_path(path);
            let _ = raw.select_value_by_path(path);
            let _ = raw.path_exists(path);
            let _ = raw.path_match(path);
        }
    }
    let key_paths = parse_key_paths(data);
    if let Ok(key_paths) = &key_paths {
        for document in &documents {
            let raw = document.as_raw();
            let _ = raw.get_by_keypath(key_paths.paths.iter());
            let _ = raw.delete_by_keypath(key_paths.paths.iter());
        }
    }

    // Formatting checks run last so that a formatting bug does not stop the
    // evaluation above from being explored.
    if let Ok(path) = json_path {
        let text = path.to_string();
        let reparsed: JsonPath<'_> = parse_json_path(text.as_bytes())
            .unwrap_or_else(|err| panic!("formatted path {text:?} does not parse: {err}"));
        assert_eq!(path, reparsed, "path changed after formatting as {text:?}");
    }
    if let Ok(key_paths) = key_paths {
        let text = key_paths.to_string();
        let reparsed: KeyPaths<'_> = parse_key_paths(text.as_bytes())
            .unwrap_or_else(|err| panic!("formatted key path {text:?} does not parse: {err}"));
        assert_eq!(
            key_paths, reparsed,
            "key path changed after formatting as {text:?}"
        );

        // The canonical form used by virtual-column metadata round-trips. It
        // has no form for the empty path, which names no virtual column.
        let owned = key_paths.to_owned();
        if !owned.paths.is_empty() {
            let canonical = owned.to_canonical_path();
            let decoded = OwnedKeyPaths::from_canonical_path(&canonical).unwrap_or_else(|err| {
                panic!("canonical path {canonical:?} does not decode: {err}")
            });
            assert_eq!(
                decoded, owned,
                "canonical path {canonical:?} changed the key path"
            );
        }
    }

    // Any canonical path that decodes encodes to a form that decodes back to
    // the same path.
    if let Ok(Ok(decoded)) = std::str::from_utf8(data).map(OwnedKeyPaths::from_canonical_path) {
        let canonical = decoded.to_canonical_path();
        assert_eq!(
            OwnedKeyPaths::from_canonical_path(&canonical).ok(),
            Some(decoded),
            "re-encoding canonical path as {canonical:?} changed it"
        );
    }
}

/// Reads a `Number` of any variant from the front of `data`: a tag byte, a
/// scale byte for decimals, then the big-endian value.
fn read_number(data: &mut &[u8]) -> Option<Number> {
    fn take<const N: usize>(data: &mut &[u8]) -> Option<[u8; N]> {
        let (head, rest) = data.split_first_chunk::<N>()?;
        *data = rest;
        Some(*head)
    }
    let [tag] = take::<1>(data)?;
    Some(match tag % 6 {
        0 => Number::Int64(i64::from_be_bytes(take(data)?)),
        1 => Number::UInt64(u64::from_be_bytes(take(data)?)),
        2 => Number::Float64(f64::from_bits(u64::from_be_bytes(take(data)?))),
        3 => {
            let [scale] = take::<1>(data)?;
            let value = i64::from_be_bytes(take(data)?);
            Number::Decimal64(Decimal64 { scale, value })
        }
        4 => {
            let [scale] = take::<1>(data)?;
            let value = i128::from_be_bytes(take(data)?);
            Number::Decimal128(Decimal128 { scale, value })
        }
        _ => {
            let [scale] = take::<1>(data)?;
            let value = ethnum::i256::from_be_bytes(take(data)?);
            Number::Decimal256(Decimal256 { scale, value })
        }
    })
}

/// The exact value of an integer `Number`.
fn integer_value(number: &Number) -> Option<i128> {
    match number {
        Number::Int64(v) => Some(i128::from(*v)),
        Number::UInt64(v) => Some(i128::from(*v)),
        _ => None,
    }
}

/// A `Number` arithmetic method.
type NumberOp = fn(&Number, Number) -> Result<Number, Error>;
/// The same operation on exact integers; `None` when it has no result.
type ExactOp = fn(i128, i128) -> Option<i128>;

/// Fuzzes two `Number`s built from arbitrary bytes.
///
/// Arithmetic, conversion, and formatting must not panic. Integer arithmetic
/// must never return a wrong value, and may fail only when the exact result
/// does not fit the result type; comparison must be antisymmetric and agree
/// with the comparable encoding; and formatted numbers must parse back to
/// equal values.
pub fn check_number(mut data: &[u8]) {
    let (Some(a), Some(b)) = (read_number(&mut data), read_number(&mut data)) else {
        return;
    };
    for n in [&a, &b] {
        check_number_unary(n);
    }

    let ops: [(&str, NumberOp, ExactOp); 5] = [
        ("add", Number::add, i128::checked_add),
        ("sub", Number::sub, i128::checked_sub),
        ("mul", Number::mul, i128::checked_mul),
        ("div", Number::div, |_, _| None),
        ("rem", Number::rem, i128::checked_rem),
    ];
    for (x, y) in [(&a, &b), (&b, &a)] {
        for (name, op, exact) in ops {
            let result = op(x, y.clone());
            let _ = result.as_ref().map(Number::to_string);
            if name == "div" {
                continue;
            }
            let (Some(xi), Some(yi)) = (integer_value(x), integer_value(y)) else {
                continue;
            };
            let expect = exact(xi, yi);
            match result {
                // A result is never silently wrong.
                Ok(got) => assert_eq!(
                    integer_value(&got),
                    expect,
                    "{x:?} {name} {y:?} returned {got:?}"
                ),
                // An error means the exact result does not fit the result
                // type: `i64` for two `Int64`, `u64` for two `UInt64`, and
                // neither for mixed operands.
                Err(err) => {
                    let fits = |v: i128, signed: bool| {
                        if signed {
                            i64::try_from(v).is_ok()
                        } else {
                            u64::try_from(v).is_ok()
                        }
                    };
                    let representable = expect.is_some_and(|v| match (x, y) {
                        (Number::Int64(_), Number::Int64(_)) => fits(v, true),
                        (Number::UInt64(_), Number::UInt64(_)) => fits(v, false),
                        _ => fits(v, true) || fits(v, false),
                    });
                    assert!(
                        !representable,
                        "{x:?} {name} {y:?} failed with {err:?}, expected {expect:?}"
                    );
                }
            }
        }
    }

    let order = a.cmp(&b);
    assert_eq!(
        order,
        b.cmp(&a).reverse(),
        "comparison is not antisymmetric: {a:?} vs {b:?}"
    );
    let comparable = |n: &Number| {
        let encoded = Value::Number(n.clone()).to_vec().expect("a number encodes");
        RawJsonb::new(&encoded).convert_to_comparable()
    };
    assert_eq!(
        order,
        comparable(&a).cmp(&comparable(&b)),
        "comparable encoding orders differently: {a:?} vs {b:?}"
    );
}

/// Checks the single-number APIs on `n`.
fn check_number_unary(n: &Number) {
    let _ = (
        n.as_i64(),
        n.as_u64(),
        n.to_i64(),
        n.to_u64(),
        n.to_i128(),
        n.as_f64(),
    );
    if let Some(v) = n.as_i128() {
        assert_eq!(
            n.cmp(&Number::Decimal128(Decimal128 { scale: 0, value: v })),
            Ordering::Equal,
            "as_i128 of {n:?} returned {v}, a different value"
        );
    }
    if let Ok(negated) = n.neg() {
        assert_eq!(
            negated.neg().map(|back| back.cmp(n)).ok(),
            Some(Ordering::Equal),
            "negating {n:?} twice changed it"
        );
    }

    let text = n.to_string();
    if n.as_f64().is_finite() {
        match parse_value(text.as_bytes()) {
            Ok(Value::Number(reparsed)) => assert_eq!(
                reparsed.cmp(n),
                Ordering::Equal,
                "{n:?} formats as {text:?}, which parses as {reparsed:?}"
            ),
            other => panic!("{n:?} formats as {text:?}, which parses as {other:?}"),
        }
    }
}

/// Fuzzes two or three values separated by `0x00` bytes.
///
/// Each part is parsed as JSON text when possible, so most inputs reach the
/// binary operations with valid JSONB; otherwise the raw bytes are used. For
/// valid values, `concat` and `contains` must match their models, and the
/// three comparisons (`RawJsonb::partial_cmp`, `Value::cmp`, and the byte
/// order of `convert_to_comparable`) must agree and form a total order.
pub fn check_binary_ops(data: &[u8]) {
    let mut parts = data.splitn(3, |&b| b == 0);
    let parts = [
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    ];
    let owned = parts.map(|part| parse_owned_jsonb(part).ok());
    let raws: Vec<RawJsonb<'_>> = parts
        .iter()
        .zip(&owned)
        .map(|(part, owned)| {
            owned
                .as_ref()
                .map_or(RawJsonb::new(part), OwnedJsonb::as_raw)
        })
        .collect();
    exercise_binary(raws[0], raws[1]);

    let valid: Vec<(RawJsonb<'_>, Value<'_>)> = owned
        .iter()
        .flatten()
        .map(|owned| {
            let raw = owned.as_raw();
            (raw, raw.to_value().expect("parsed JSONB decodes"))
        })
        .collect();
    for (a, a_value) in &valid {
        for (b, b_value) in &valid {
            check_pair(*a, a_value, *b, b_value);
        }
    }
    if let [a, b, c] = valid.as_slice() {
        let le = |x: &RawJsonb<'_>, y: &RawJsonb<'_>| x.partial_cmp(y) != Some(Ordering::Greater);
        for [x, y, z] in [
            [a, b, c],
            [a, c, b],
            [b, a, c],
            [b, c, a],
            [c, a, b],
            [c, b, a],
        ] {
            assert!(
                !(le(&x.0, &y.0) && le(&y.0, &z.0)) || le(&x.0, &z.0),
                "comparison is not transitive: {} <= {} <= {} but not {} <= {}",
                x.1,
                y.1,
                z.1,
                x.1,
                z.1
            );
        }
    }
}

/// Checks two valid values against each other.
fn check_pair(a: RawJsonb<'_>, a_value: &Value<'_>, b: RawJsonb<'_>, b_value: &Value<'_>) {
    let order = a.partial_cmp(&b);
    assert_eq!(
        order,
        b.partial_cmp(&a).map(Ordering::reverse),
        "comparison is not antisymmetric: {a_value} vs {b_value}"
    );
    assert!(
        order.is_some(),
        "valid values must compare: {a_value} vs {b_value}"
    );
    assert_eq!(
        order,
        Some(a_value.cmp(b_value)),
        "RawJsonb and Value order differ: {a_value} vs {b_value}"
    );
    assert_eq!(
        order,
        Some(a.convert_to_comparable().cmp(&b.convert_to_comparable())),
        "comparable encoding orders differently: {a_value} vs {b_value}"
    );

    assert_model(
        || format!("concat({a_value}, {b_value})"),
        a.concat(&b).ok(),
        Some(model::concat(a_value, b_value)),
    );
    assert_eq!(
        a.contains(&b).expect("contains"),
        model::contains(a_value, b_value),
        "contains({a_value}, {b_value})"
    );
}
