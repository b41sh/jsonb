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

//! Regression tests for malformed JSONB bytes, deep nesting, and oversized headers.
//!
//! Every public API must return an error (or a documented fallback) for such
//! input instead of panicking, overflowing the stack, or allocating memory
//! proportional to a length field instead of the input size.

use std::panic::{self, AssertUnwindSafe};

use ethnum::I256;
use jsonb::jsonpath::parse_json_path;
use jsonb::keypath::parse_key_paths;
use jsonb::*;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

/// Nesting limit for arrays and objects (`MAX_NESTING_DEPTH` in the crate).
const MAX_DEPTH: usize = 512;
/// Parenthesis nesting limit for JSONPath (`MAX_JSON_PATH_DEPTH` in the crate).
const MAX_PATH_DEPTH: usize = 64;

/// Runs `f` on a thread with a large stack.
///
/// Debug builds use up to ~9 KiB of stack per nesting level, so walking
/// `MAX_DEPTH` levels needs more than the test harness's default 2 MiB.
fn with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

/// Builds `[[...[]...]]` with `depth` arrays, using the public builder.
fn nested_arrays(depth: usize) -> OwnedJsonb {
    let mut cur = "[]".parse::<OwnedJsonb>().unwrap();
    for _ in 1..depth {
        cur = OwnedJsonb::build_array([cur.as_raw()]).unwrap();
    }
    cur
}

/// Builds `{"k":{"k":...{}...}}` with `depth` objects, using the public builder.
fn nested_objects(depth: usize) -> OwnedJsonb {
    let mut cur = "{}".parse::<OwnedJsonb>().unwrap();
    for _ in 1..depth {
        cur = OwnedJsonb::build_object([("k", cur.as_raw())]).unwrap();
    }
    cur
}

fn jsonb(words: &[u32], tail: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    out.extend_from_slice(tail);
    out
}

#[test]
fn malformed_number_payloads_return_errors() {
    let cases = [
        ("empty number", jsonb(&[0x2000_0000, 0x2000_0000], &[])),
        (
            "float with 1 byte",
            jsonb(&[0x2000_0000, 0x2000_0002], &[0x60, 0xf9]),
        ),
        ("truncated payload", jsonb(&[0x2000_0000, 0x2000_0001], &[])),
    ];
    for (name, bytes) in &cases {
        let raw = RawJsonb::new(bytes);
        assert!(raw.as_number().is_err(), "{name}: as_number");
        assert!(raw.to_value().is_err(), "{name}: to_value");
        assert!(
            from_raw_jsonb::<i64>(&raw).is_err(),
            "{name}: from_raw_jsonb"
        );
        assert!(serde_json::to_string(&raw).is_err(), "{name}: serialize");
        assert!(from_slice(bytes).is_err(), "{name}: from_slice");
        // `to_string` falls back to treating the bytes as text.
        assert_eq!(raw.to_string(), String::from_utf8_lossy(bytes), "{name}");
    }
}

#[test]
fn malformed_extension_payloads_return_errors() {
    let cases = [
        ("empty extension", jsonb(&[0x2000_0000, 0x6000_0000], &[])),
        (
            "truncated extension",
            jsonb(&[0x2000_0000, 0x6000_008f], &[]),
        ),
    ];
    for (name, bytes) in &cases {
        let raw = RawJsonb::new(bytes);
        assert!(
            raw.as_extension_value().is_err(),
            "{name}: as_extension_value"
        );
        assert!(raw.to_value().is_err(), "{name}: to_value");
        assert!(
            from_raw_jsonb::<String>(&raw).is_err(),
            "{name}: from_raw_jsonb"
        );
        assert!(serde_json::to_string(&raw).is_err(), "{name}: serialize");
    }
}

#[test]
fn truncated_container_payloads_return_errors() {
    // An array whose container element claims more bytes than remain.
    let bytes = jsonb(&[0x8000_0001, 0x5000_0010], &[0x80, 0, 0, 0]);
    let raw = RawJsonb::new(&bytes);
    assert!(serde_json::to_string(&raw).is_err());
    assert!(raw.to_value().is_err());
}

#[test]
fn from_slice_rejects_non_string_object_key() {
    // `{null: "v"}`
    let bytes = jsonb(&[0x4000_0001, 0x0000_0000, 0x1000_0001], b"v");
    assert!(from_slice(&bytes).is_err());
}

#[test]
fn display_does_not_panic_on_out_of_range_values() {
    let values = [
        Value::Date(Date { value: i32::MAX }),
        Value::Date(Date { value: i32::MIN }),
        Value::TimestampTz(TimestampTz {
            offset: i32::MAX,
            value: 0,
        }),
        Value::TimestampTz(TimestampTz {
            offset: i32::MIN,
            value: i64::MIN,
        }),
        Value::Interval(Interval {
            months: i32::MIN,
            days: i32::MIN,
            micros: i64::MIN,
        }),
    ];
    for value in &values {
        let mut buf = Vec::new();
        value.write_to_vec(&mut buf).unwrap();
        let raw = RawJsonb::new(&buf);
        assert!(!raw.to_string().is_empty(), "{value:?}");
        assert!(from_raw_jsonb::<String>(&raw).is_ok(), "{value:?}");
    }
    let interval = Interval {
        months: 0,
        days: i32::MIN,
        micros: i64::MIN,
    };
    assert_eq!(
        interval.to_string(),
        "-2147483648 days -2562047788:00:54.775808"
    );
}

#[test]
fn decimal_display_handles_any_scale_and_value() {
    // 2^255 - 1 and 2^255, as literals so the test does not depend on `ethnum`'s Display.
    let i256_max = "57896044618658097711785492504343953926634992332820282019728792003956564819967";
    let i256_min_abs =
        "57896044618658097711785492504343953926634992332820282019728792003956564819968";
    let (int, frac) = i256_min_abs.split_at(i256_min_abs.len() - 3);
    let cases = vec![
        (
            Number::Decimal128(Decimal128 {
                scale: 2,
                value: 12345,
            }),
            "123.45".to_string(),
        ),
        (
            Number::Decimal128(Decimal128 {
                scale: 40,
                value: 5,
            }),
            format!("0.{}5", "0".repeat(39)),
        ),
        (
            Number::Decimal128(Decimal128 {
                scale: 0,
                value: i128::MIN,
            }),
            i128::MIN.to_string(),
        ),
        (
            Number::Decimal128(Decimal128 {
                scale: 39,
                value: i128::MIN,
            }),
            "-0.170141183460469231731687303715884105728".to_string(),
        ),
        (
            Number::Decimal64(Decimal64 {
                scale: 200,
                value: -1,
            }),
            format!("-0.{}1", "0".repeat(199)),
        ),
        (
            Number::Decimal256(Decimal256 {
                scale: 0,
                value: I256::MAX,
            }),
            i256_max.to_string(),
        ),
        (
            Number::Decimal256(Decimal256 {
                scale: 3,
                value: I256::MIN,
            }),
            format!("-{int}.{frac}"),
        ),
        (
            // Below the fast path's 10^76 bound, but a scale beyond 76 digits;
            // the low 10^38 chunk needs zero padding.
            Number::Decimal256(Decimal256 {
                scale: 80,
                value: I256::from(10_i128.pow(37)) * I256::from(70_i128) + I256::from(42_i128),
            }),
            format!("0.{}7{}42", "0".repeat(41), "0".repeat(36)),
        ),
    ];
    for (number, expected) in &cases {
        assert_eq!(&number.to_string(), expected, "{number:?}");
    }
}

#[test]
fn decimal_comparison_with_huge_scale_difference_does_not_panic() {
    let small = Number::Decimal128(Decimal128 { scale: 0, value: 1 });
    let tiny = Number::Decimal128(Decimal128 {
        scale: 200,
        value: 1,
    });
    assert_eq!(small.partial_cmp(&tiny), Some(std::cmp::Ordering::Greater));
    let small = Number::Decimal256(Decimal256 {
        scale: 0,
        value: I256::ONE,
    });
    let tiny = Number::Decimal256(Decimal256 {
        scale: 200,
        value: I256::ONE,
    });
    assert_eq!(tiny.partial_cmp(&small), Some(std::cmp::Ordering::Less));
}

#[test]
fn json_path_rejects_unterminated_string() {
    for path in [r#"""#, r#""a"#, r#"$."a"#, r#"$["a"#] {
        assert!(parse_json_path(path.as_bytes()).is_err(), "{path}");
        assert!(parse_key_paths(path.as_bytes()).is_err(), "{path}");
    }
}

/// Encodes `depth` nested arrays directly, in linear time, for very deep inputs.
fn deep_arrays_bytes(depth: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(depth * 8);
    for level in 0..depth - 1 {
        // The child container has `depth - level - 2` containers below it.
        let child_len = 8 * (depth - level - 2) + 4;
        out.extend_from_slice(&0x8000_0001_u32.to_be_bytes());
        out.extend_from_slice(&(0x5000_0000_u32 | child_len as u32).to_be_bytes());
    }
    out.extend_from_slice(&0x8000_0000_u32.to_be_bytes());
    out
}

#[test]
fn deep_arrays_bytes_matches_builder() {
    assert_eq!(deep_arrays_bytes(5), nested_arrays(5).to_vec());
}

#[test]
fn text_parsers_enforce_nesting_limit() {
    with_big_stack(|| {
        for (open, empty, close) in [("[", "[]", "]"), (r#"{"a":"#, "{}", "}")] {
            let text = |depth: usize| {
                let outer = depth - 1;
                format!("{}{empty}{}", open.repeat(outer), close.repeat(outer))
            };
            let ok = text(MAX_DEPTH);
            assert!(parse_value(ok.as_bytes()).is_ok());
            assert!(parse_value_standard_mode(ok.as_bytes()).is_ok());
            assert!(parse_owned_jsonb(ok.as_bytes()).is_ok());
            for depth in [MAX_DEPTH + 1, 100_000] {
                let deep = text(depth);
                let err = Some(Error::RecursionLimitExceeded);
                assert_eq!(parse_value(deep.as_bytes()).err(), err, "{open} x {depth}");
                assert_eq!(parse_value_standard_mode(deep.as_bytes()).err(), err);
                assert_eq!(parse_owned_jsonb(deep.as_bytes()).err(), err);
                assert_eq!(deep.parse::<OwnedJsonb>().err(), err);
            }
        }
    });
}

/// Runs every recursive read API on `raw` and reports whether each succeeded.
fn recursive_api_results(raw: RawJsonb<'_>) -> Vec<(&'static str, bool)> {
    let all = parse_json_path(b"$.**").unwrap();
    vec![
        ("to_value", raw.to_value().is_ok()),
        ("to_pretty_string", raw.to_pretty_string() != "null"),
        ("serde_json::to_string", serde_json::to_string(&raw).is_ok()),
        ("from_slice", from_slice(raw.as_ref()).is_ok()),
        (
            "from_raw_jsonb",
            from_raw_jsonb::<serde_json::Value>(&raw).is_ok(),
        ),
        ("partial_cmp", raw.partial_cmp(&raw).is_some()),
        ("contains", raw.contains(&raw).is_ok()),
        ("strip_nulls", raw.strip_nulls().is_ok()),
        (
            "convert_to_comparable",
            !raw.convert_to_comparable().is_empty(),
        ),
        ("select_by_path", raw.select_by_path(&all).is_ok()),
        ("jaq", jaq_json::Val::try_from(raw).is_ok()),
        (
            "visit_scalar_key_paths",
            raw.visit_scalar_key_paths(false, |_| Ok(())).is_ok(),
        ),
    ]
}

#[test]
fn binary_apis_enforce_nesting_limit() {
    with_big_stack(|| {
        for (shape, build) in [
            ("arrays", nested_arrays as fn(usize) -> OwnedJsonb),
            ("objects", nested_objects),
        ] {
            let ok = build(MAX_DEPTH);
            for (api, passed) in recursive_api_results(ok.as_raw()) {
                assert!(passed, "{api} should accept {MAX_DEPTH} nested {shape}");
            }
            let deep = build(MAX_DEPTH + 1);
            for (api, passed) in recursive_api_results(deep.as_raw()) {
                assert!(
                    !passed,
                    "{api} should reject {} nested {shape}",
                    MAX_DEPTH + 1
                );
            }
            assert_eq!(
                deep.as_raw().to_value().err(),
                Some(Error::RecursionLimitExceeded)
            );
        }
    });
}

#[test]
fn binary_apis_reject_very_deep_input() {
    // Without the limit, 100000 levels need far more than the 64 MiB test stack.
    with_big_stack(|| {
        let bytes = deep_arrays_bytes(100_000);
        for (api, passed) in recursive_api_results(RawJsonb::new(&bytes)) {
            assert!(!passed, "{api} should reject 100000 nested arrays");
        }
    });
}

#[test]
fn json_path_enforces_paren_limit() {
    let path = |depth: usize| {
        let inner = depth - 1;
        format!("$ ? ({}@ == 1{})", "(".repeat(inner), ")".repeat(inner))
    };
    with_big_stack(move || {
        assert!(parse_json_path(path(MAX_PATH_DEPTH).as_bytes()).is_ok());
    });
    for depth in [MAX_PATH_DEPTH + 1, 100_000] {
        assert_eq!(
            parse_json_path(path(depth).as_bytes()).err(),
            Some(Error::RecursionLimitExceeded),
            "depth {depth}"
        );
    }
    // Parentheses inside string literals do not count.
    let quoted = format!(r#"$ ? (@ == "{}")"#, "(".repeat(1000));
    assert!(parse_json_path(quoted.as_bytes()).is_ok());
}

#[test]
fn recursive_descent_reaches_levels_beyond_255() {
    with_big_stack(|| {
        // Levels are `u8`, so an open `to last` range is the only way to match
        // past level 255; the innermost `[]` of 300 nested arrays is level 299.
        let deep = nested_arrays(300);
        let path = parse_json_path(b"$.**{255 to last}").unwrap();
        let selected = deep.as_raw().select_by_path(&path).unwrap();
        assert_eq!(selected.len(), 300 - 255);
        assert_eq!(selected.last().unwrap().to_string(), "[]");
    });
}

#[test]
fn oversized_container_headers_are_rejected_early() {
    // Each header declares 2^29 - 1 entries but no JEntries follow.
    for header in [0x9FFF_FFFF_u32, 0x5FFF_FFFF] {
        let bytes = header.to_be_bytes();
        let raw = RawJsonb::new(&bytes);
        assert_eq!(raw.to_value().err(), Some(Error::InvalidEOF), "{header:#x}");
        assert!(raw.array_length().is_err(), "{header:#x}");
        assert!(raw.object_keys().is_err(), "{header:#x}");
        assert!(
            from_raw_jsonb::<serde_json::Value>(&raw).is_err(),
            "{header:#x}"
        );
        assert!(serde_json::to_value(raw).is_err(), "{header:#x}");
        assert!(from_slice(&bytes).is_err(), "{header:#x}");
    }
}

fn mutation_corpus() -> Vec<Vec<u8>> {
    let texts = [
        "null",
        "true",
        "-7",
        "123456",
        "-1.5",
        "1e300",
        "123456789012345678901234567890.5",
        r#""测试""#,
        r#"[1,"a",null,{"k":[true,2.5]}]"#,
        r#"{"a":{"b":[1,2,{"c":"d"}]},"e":3.25,"f":null}"#,
        r#"[[[[["x"]]]],{"y":{"z":[]}}]"#,
    ];
    let mut corpus: Vec<Vec<u8>> = texts
        .iter()
        .map(|t| t.parse::<OwnedJsonb>().unwrap().to_vec())
        .collect();
    for value in [
        Value::Date(Date { value: 19000 }),
        Value::TimestampTz(TimestampTz {
            offset: 3600,
            value: 1,
        }),
        Value::Interval(Interval {
            months: 1,
            days: 2,
            micros: 3,
        }),
        Value::Array(vec![Value::Binary(b"z"), Value::Date(Date { value: 1 })]),
    ] {
        let mut buf = Vec::new();
        value.write_to_vec(&mut buf).unwrap();
        corpus.push(buf);
    }
    corpus
}

/// Applies a few random byte edits, biased towards JEntry/header words.
fn mutate(rng: &mut SmallRng, seed: &[u8]) -> Vec<u8> {
    const WORDS: [u32; 8] = [
        0x2000_0000,
        0x6000_0000,
        0x7000_0000,
        0x5000_0004,
        0x1000_0000,
        0x2000_0001,
        0x8000_0002,
        0x9FFF_FFFF,
    ];
    let mut v = seed.to_vec();
    for _ in 0..rng.random_range(1..4) {
        let n = v.len().max(1);
        match rng.random_range(0..5) {
            0 if !v.is_empty() => {
                let i = rng.random_range(0..v.len());
                v[i] = rng.random();
            }
            1 => v.truncate(rng.random_range(0..n)),
            2 => {
                let i = rng.random_range(0..=v.len());
                v.insert(i, rng.random());
            }
            3 if v.len() >= 4 => {
                let i = rng.random_range(0..=(v.len() - 4)) & !3;
                let w = WORDS[rng.random_range(0..WORDS.len())];
                v[i..i + 4].copy_from_slice(&w.to_be_bytes());
            }
            _ => {}
        }
    }
    v
}

#[test]
fn mutated_jsonb_never_panics() {
    let corpus = mutation_corpus();
    let other = r#"[1,{"a":"b"}]"#.parse::<OwnedJsonb>().unwrap();
    let other = other.as_raw();
    let keys = ["a", "k"].into_iter().collect();
    let path = parse_json_path(b"$.a[*].b").unwrap();
    let mut rng = SmallRng::seed_from_u64(0x5eed);
    for _ in 0..3000 {
        let seed = &corpus[rng.random_range(0..corpus.len())];
        let bytes = mutate(&mut rng, seed);
        let raw = RawJsonb::new(&bytes);
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = from_slice(&bytes);
            let _ = raw.to_value();
            let _ = raw.to_string();
            let _ = raw.to_pretty_string();
            let _ = from_raw_jsonb::<serde_json::Value>(&raw);
            let _ = from_raw_jsonb::<String>(&raw);
            let _ = from_raw_jsonb::<f64>(&raw);
            let _ = raw.type_of();
            let _ = raw.to_str();
            let _ = raw.as_number();
            let _ = raw.as_extension_value();
            let _ = raw.array_values();
            let _ = raw.array_distinct();
            let _ = raw.array_intersection(&other);
            let _ = raw.array_overlap(&other);
            let _ = raw.object_each();
            let _ = raw.object_keys();
            let _ = raw.object_pick(&keys);
            let _ = raw.get_by_index(1);
            let _ = raw.get_by_name("a", true);
            let _ = raw.strip_nulls();
            let _ = raw.concat(&other);
            let _ = raw.contains(&other);
            let _ = other.contains(&raw);
            let _ = raw.convert_to_comparable();
            let _ = raw.partial_cmp(&other);
            let _ = raw.select_by_path(&path);
            let _ = OwnedJsonb::build_array([raw, other]).map(|o| o.to_string());
        }));
        assert!(result.is_ok(), "panicked on input {bytes:02x?}");
    }
}

#[test]
fn encoding_rejects_strings_that_overflow_the_length_field() {
    // JEntry lengths have 28 bits, so a 256 MiB string cannot be encoded.
    let big = String::from_utf8(vec![b'a'; 1 << 28]).unwrap();
    let value = Value::String(big.as_str().into());
    assert_eq!(value.to_vec().err(), Some(Error::ValueTooLarge));

    // A failed `write_to_vec` leaves earlier buffer contents untouched.
    let mut buf = b"prefix".to_vec();
    let nested = Value::Array(vec![Value::Null, value]);
    assert_eq!(nested.write_to_vec(&mut buf), Err(Error::ValueTooLarge));
    assert_eq!(buf, b"prefix");

    let raw = RawJsonb::new(b"\x20\0\0\0\0\0\0\0");
    assert_eq!(
        OwnedJsonb::build_object([(big.as_str(), raw)]).err(),
        Some(Error::ValueTooLarge)
    );
    assert_eq!(to_owned_jsonb(&big).err(), Some(Error::ValueTooLarge));

    // One byte below the limit still round-trips.
    let fits = &big[..(1 << 28) - 1];
    let encoded = Value::String(fits.into()).to_vec().unwrap();
    assert_eq!(
        RawJsonb::new(&encoded).as_str().unwrap().unwrap().len(),
        fits.len()
    );
}

#[test]
fn comparable_encoding_orders_values_past_255_levels() {
    with_big_stack(|| {
        // `a` = [X, 3] and `b` = [Y], where X and Y are 256 nested arrays whose
        // innermost arrays are `[]` and `[2]`. `a < b` because `[]` < `[2]`.
        // A `u8` depth wrapped 257 to 1, so the old encoding compared `3`
        // against `2` and ordered `a > b`.
        let nest = |inner: &str| format!("{}{inner}{}", "[".repeat(255), "]".repeat(255));
        let a = format!("[{},3]", nest("[]")).parse::<OwnedJsonb>().unwrap();
        let b = format!("[{}]", nest("[2]")).parse::<OwnedJsonb>().unwrap();
        assert_eq!(
            a.as_raw().partial_cmp(&b.as_raw()),
            Some(std::cmp::Ordering::Less)
        );
        assert!(a.as_raw().convert_to_comparable() < b.as_raw().convert_to_comparable());
    });
}

#[test]
fn deserializing_skips_unknown_fields_and_ignored_elements() {
    use serde::de::IgnoredAny;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Partial {
        b: String,
    }

    // Unknown fields `a` and `c`, including nested containers, are skipped.
    let object = r#"{"a":[1,{"x":2}],"b":"y","c":null}"#.parse::<OwnedJsonb>().unwrap();
    let partial: Partial = from_raw_jsonb(&object.as_raw()).unwrap();
    assert_eq!(partial, Partial { b: "y".to_string() });

    // Array elements are read in order, so skipping one must advance past it.
    let array = r#"[[1,[2]],"z",{"k":"v"},3]"#.parse::<OwnedJsonb>().unwrap();
    let (_, second, _, fourth): (IgnoredAny, String, IgnoredAny, u8) =
        from_raw_jsonb(&array.as_raw()).unwrap();
    assert_eq!((second.as_str(), fourth), ("z", 3));

    // A whole value can be ignored too.
    assert!(from_raw_jsonb::<IgnoredAny>(&array.as_raw()).is_ok());
}

#[test]
fn signed_indexes_do_not_overflow() {
    let array = "[1,2,3]".parse::<OwnedJsonb>().unwrap();
    let raw = array.as_raw();
    assert_eq!(
        raw.delete_by_index(i32::MIN).unwrap().to_string(),
        "[1,2,3]"
    );
    assert_eq!(raw.delete_by_index(-1).unwrap().to_string(), "[1,2]");
    let one = "1".parse::<OwnedJsonb>().unwrap();
    assert_eq!(
        raw.array_insert(i32::MIN, &one.as_raw())
            .unwrap()
            .to_string(),
        "[1,1,2,3]"
    );
    assert_eq!(
        raw.array_insert(i32::MAX, &one.as_raw())
            .unwrap()
            .to_string(),
        "[1,2,3,1]"
    );
    for path in [
        "$[last + 2147483647]",
        "$[last - 2147483647]",
        "$[-2147483648]",
    ] {
        let path = parse_json_path(path.as_bytes()).unwrap();
        assert!(raw.select_by_path(&path).unwrap().is_empty());
    }
}

#[test]
fn json_path_slices_outside_the_array_select_nothing() {
    // These used to loop over `0..=usize::MAX` after an end index underflowed.
    let array = "[1,2]".parse::<OwnedJsonb>().unwrap();
    let empty = "[]".parse::<OwnedJsonb>().unwrap();
    for (doc, path) in [
        (&array, "$[0 to last - 5]"),
        (&empty, "$[0 to 3]"),
        (&array, "$[5 to 9]"),
    ] {
        let path = parse_json_path(path.as_bytes()).unwrap();
        assert!(doc.as_raw().select_by_path(&path).unwrap().is_empty());
    }
    let path = parse_json_path(b"$[1 to 9]").unwrap();
    let selected = array.as_raw().select_by_path(&path).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].to_string(), "2");
}

#[test]
fn path_display_round_trips_special_names() {
    // Names that need quoting or escaping when shown. The parsers reject empty
    // quoted names, so `""` is not included.
    for name in [
        r#"st"ore"#,
        "a b",
        "a.b",
        "a\\b",
        "$x",
        "1st",
        "tab\there",
    ] {
        let quoted = serde_json::to_string(name).unwrap();
        for text in [
            format!("$.{quoted}"),
            format!("$[{quoted}]"),
            format!("$:{quoted}"),
            format!("a:{quoted}"),
        ] {
            let path = parse_json_path(text.as_bytes()).unwrap();
            let shown = path.to_string();
            assert_eq!(
                parse_json_path(shown.as_bytes()).unwrap(),
                path,
                "{text} shown as {shown}"
            );
        }
        let text = format!("{{{quoted},0}}");
        let key_paths = parse_key_paths(text.as_bytes()).unwrap();
        let shown = key_paths.to_string();
        assert_eq!(
            parse_key_paths(shown.as_bytes()).unwrap(),
            key_paths,
            "{text} shown as {shown}"
        );
    }
}
