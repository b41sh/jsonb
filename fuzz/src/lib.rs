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

use jsonb::jsonpath::parse_json_path;
use jsonb::jsonpath::JsonPath;
use jsonb::keypath::parse_key_paths;
use jsonb::keypath::KeyPaths;
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
}

/// A JSON text parser that builds JSONB directly, paired with the one that
/// builds a `Value` in the same mode.
type ParserPair = (
    fn(&[u8]) -> Result<OwnedJsonb, Error>,
    fn(&[u8]) -> Result<Value<'_>, Error>,
);

/// Fuzzes arbitrary bytes interpreted as JSON text.
///
/// Both encoders (`parse_owned_jsonb` and `parse_value` + `to_vec`) must agree
/// byte for byte, and formatting the result must parse back to an equal value.
pub fn check_json_text(data: &[u8]) {
    let parsers: [ParserPair; 2] = [
        (parse_owned_jsonb, parse_value),
        (parse_owned_jsonb_standard_mode, parse_value_standard_mode),
    ];
    for (parse_owned, parse) in parsers {
        let owned = parse_owned(data);
        let value = parse(data);
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

        let raw = owned.as_raw();
        check_canonical(raw);
        exercise_unary(raw);

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
    }
}

/// Fuzzes two values split at the first `0x00` byte.
///
/// Each half is parsed as JSON text when possible, so most inputs reach the
/// binary operations with valid JSONB; otherwise the raw bytes are used.
/// Comparison of valid values must be antisymmetric.
pub fn check_binary_ops(data: &[u8]) {
    let (left, right) = match data.iter().position(|&b| b == 0) {
        Some(pos) => (&data[..pos], &data[pos + 1..]),
        None => (data, &[][..]),
    };
    let left_owned = parse_owned_jsonb(left).ok();
    let right_owned = parse_owned_jsonb(right).ok();
    let left_raw = left_owned
        .as_ref()
        .map_or(RawJsonb::new(left), OwnedJsonb::as_raw);
    let right_raw = right_owned
        .as_ref()
        .map_or(RawJsonb::new(right), OwnedJsonb::as_raw);
    exercise_binary(left_raw, right_raw);

    if left_owned.is_some() && right_owned.is_some() {
        let forward = left_raw.partial_cmp(&right_raw);
        let backward = right_raw.partial_cmp(&left_raw);
        assert_eq!(
            forward,
            backward.map(Ordering::reverse),
            "comparison is not antisymmetric"
        );
    }
}
