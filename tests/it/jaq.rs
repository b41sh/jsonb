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

use jaq_core::load::Arena;
use jaq_core::load::File;
use jaq_core::load::Loader;
use jaq_core::unwrap_valr;
use jaq_core::Compiler;
use jaq_core::Ctx;
use jaq_core::Vars;

use jsonb::jaq::all_defs;
use jsonb::jaq::all_funs;
use jsonb::jaq::jaq_val_to_owned_jsonb;
use jsonb::jaq::json_val_defs;
use jsonb::jaq::json_val_funs;
use jsonb::jaq::raw_jsonb_to_jaq_val;
use jsonb::jaq::JsonValData;
use jsonb::jaq::JsonbData;
use jsonb::jaq::QueryValue;
use jsonb::OwnedJsonb;

fn try_run_filter(filter: &'static str, input: &str) -> Result<Vec<String>, String> {
    try_run_filter_with(filter, input, |value| Ok(value.to_string()))
}

fn try_run_jsonb_filter(filter: &'static str, input: &str) -> Result<Vec<String>, String> {
    try_run_filter_with(filter, input, |value| {
        value
            .into_owned_jsonb()
            .map(|value| value.as_raw().to_string())
            .map_err(|err| err.to_string())
    })
}

fn try_run_filter_with(
    filter: &'static str,
    input: &str,
    output: impl Fn(QueryValue<'_>) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let arena = Arena::default();
    let loader = Loader::new(all_defs());
    let modules = loader
        .load(
            &arena,
            File {
                path: (),
                code: filter,
            },
        )
        .map_err(|errors| format!("{errors:?}"))?;
    let filter = Compiler::default()
        .with_funs(all_funs::<JsonbData>())
        .compile(modules)
        .map_err(|errors| format!("{errors:?}"))?;

    let input_jsonb = input.parse::<OwnedJsonb>().map_err(|err| err.to_string())?;
    let input = QueryValue::from_raw(input_jsonb.as_raw());
    let ctx = Ctx::<JsonbData>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input))
        .map(unwrap_valr)
        .map(|value| value.map_err(|err| err.to_string()).and_then(&output))
        .collect()
}

fn try_run_jaq_json_filter(filter: &'static str, input: &str) -> Result<Vec<String>, String> {
    let arena = Arena::default();
    let loader = Loader::new(json_val_defs());
    let modules = loader
        .load(
            &arena,
            File {
                path: (),
                code: filter,
            },
        )
        .map_err(|errors| format!("{errors:?}"))?;
    let filter = Compiler::default()
        .with_funs(json_val_funs())
        .compile(modules)
        .map_err(|errors| format!("{errors:?}"))?;

    let input_jsonb = input.parse::<OwnedJsonb>().map_err(|err| err.to_string())?;
    let input = raw_jsonb_to_jaq_val(input_jsonb.as_raw()).map_err(|err| err.to_string())?;
    let ctx = Ctx::<JsonValData>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input))
        .map(unwrap_valr)
        .map(|value| {
            value
                .map_err(|err| err.to_string())
                .and_then(|value| jaq_val_to_owned_jsonb(&value).map_err(|err| err.to_string()))
                .map(|value| value.as_raw().to_string())
        })
        .collect()
}

fn give(input: &str, filter: &'static str, output: &str) {
    gives(input, filter, &[output]);
}

fn gives(input: &str, filter: &'static str, outputs: &[&str]) {
    let outputs = outputs
        .iter()
        .map(|output| output.to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        try_run_filter(filter, input),
        Ok(outputs),
        "input: {input}, filter: {filter}"
    );
}

fn fail(input: &str, filter: &'static str, error: &str) {
    assert_eq!(
        try_run_filter(filter, input),
        Err(error.to_string()),
        "input: {input}, filter: {filter}"
    );
}

#[test]
fn jaq_profile_projection_and_array_index_regression() {
    let input = r#"{"account_balance":8719.17,"address":{"city":"City64","country":"Country4","postal_code":"10664","state":"State4","street":"58337f64a7 St"},"avatar":"avatar1666664.jpg","birthday":"1990-01-01","country_code":"CN","created_at":"2026-06-05 07:12:24.609283","devices":[{"device_id":"device166666401","os":"iOS","type":"mobile"},{"device_id":"device166666402","os":"Windows","type":"laptop"}],"email":"user1666664@example.com","id":"1666664","ip_address":"192.168.104.1","last_purchase":{"amount":1609.23,"date":"2026-06-05 07:12:24.609283","item":"Laptop"},"loyalty_points":"16666640","membership_tier":"Gold","nickname":"user1666664","phone":"******7664","preferences":{"newsletter_opt_in":true,"timezone":"UTC+8"},"purchase_history":[{"amount":799.99,"date":"2023-12-15T10:00:00Z","item":"Phone"},{"amount":199.99,"date":"2023-11-20T15:00:00Z","item":"Headphones"}],"recent_searches":["laptops","smartphones","headphones"],"status":"2","subscription_expiration":"2025-01-01","subscription_status":"active","updated_at":"2026-06-09 07:12:24.609283","valid":true,"wishlist":["Smartwatch","Tablet"]}"#;

    give(
        input,
        r#"{account_id: .id, balance: .account_balance, loyalty_points: (.loyalty_points | tonumber), valid, expired: .subscription_status == "inactive"}"#,
        r#"{"account_id":"1666664","balance":8719.17,"expired":false,"loyalty_points":16666640,"valid":true}"#,
    );
    give(input, r#".recent_searches | index("smartphones")"#, "1");

    assert_eq!(
        try_run_jsonb_filter(
            r#"{account_id: .id, balance: .account_balance, loyalty_points: (.loyalty_points | tonumber), valid, expired: .subscription_status == "inactive"}"#,
            input
        ),
        Ok(vec![
            r#"{"account_id":"1666664","balance":8719.17,"expired":false,"loyalty_points":16666640,"valid":true}"#.to_string()
        ])
    );
    assert_eq!(
        try_run_jsonb_filter(r#".recent_searches | index("smartphones")"#, input),
        Ok(vec!["1".to_string()])
    );
}

#[test]
fn jaq_json_val_conversion_runs_filters() {
    let input = r#"{"account_balance":8719.17,"id":"1666664","loyalty_points":"16666640","recent_searches":["laptops","smartphones","headphones"],"subscription_status":"active","valid":true}"#;

    assert_eq!(
        try_run_jaq_json_filter(
            r#"{account_id: .id, balance: .account_balance, loyalty_points: (.loyalty_points | tonumber), valid, expired: .subscription_status == "inactive"}"#,
            input
        ),
        Ok(vec![
            r#"{"account_id":"1666664","balance":8719.17,"expired":false,"loyalty_points":16666640,"valid":true}"#.to_string()
        ])
    );
    assert_eq!(
        try_run_jaq_json_filter(r#".recent_searches | index("smartphones")"#, input),
        Ok(vec!["1".to_string()])
    );
}

// Ported from jaq/jaq-json/tests/funs.rs.
#[test]
fn jaq_json_funs_compat() {
    give("null", "[1, 3] | bsearch(0)", "-1");
    give("null", "[1, 3] | bsearch(2)", "-2");
    give("null", "[1, 3] | bsearch(4)", "-3");
    give("null", "[1, 3] | [bsearch(1, 3)]", "[0,1]");

    give(
        "null",
        r#""Infinity +Infinity -Infinity" | [fromjson | tostring]"#,
        r#"["Infinity","Infinity","-Infinity"]"#,
    );
    give("null", r#"" 1" | fromjson"#, "1");
    give("null", r#""+1" | fromjson"#, "1");
    give("null", r#""-1" | fromjson"#, "-1");

    give("[0, null]", "has(0)", "true");
    give("[0, null]", "has(1)", "true");
    give("[0, null]", "has(2)", "false");
    give(r#"{"a": 1, "b": null}"#, r#"has("a")"#, "true");
    give(r#"{"a": 1, "b": null}"#, r#"has("b")"#, "true");
    give(r#"{"a": 1, "b": null}"#, r#"has("c")"#, "false");

    give("null", r#""a,b, cd, efg" | indices(", ")"#, "[3,7]");
    give("null", "[0, 1, 2, 1, 3, 1, 4] | indices(1)", "[1,3,5]");
    give(
        "null",
        "[0, 1, 2, 3, 1, 4, 2, 5, 1, 2, 6, 7] | indices([1, 2])",
        "[1,8]",
    );
    give("null", r#"["a", "b", "c"] | indices("b")"#, "[1]");
    give("null", "[0, 1] | indices([])", "[]");
    give("null", "[1, 2] | indices([1, 2, 3])", "[]");
    give("null", "[0, 0, 0] | indices([0, 0])", "[0,1]");
    give("null", r#""aaa" | indices("aa")"#, "[0,1]");
    give("null", r#""🇬🇧!" | indices("!")"#, "[2]");
    give("null", r#""🇬🇧🇬🇧" | indices("🇬🇧")"#, "[0,2]");

    give("null", r#""ƒoo" | length"#, "3");
    give("null", r#""नमस्ते" | length"#, "6");
    give("null", r#"{"a": 5, "b": 3} | length"#, "2");
    give("null", " 2 | length", "2");
    give("null", "-2 | length", "2");
    give("null", " 2.5 | length", "2.5");
    give("null", "-2.5 | length", "2.5");

    give("null", "1.0 | tojson", r#""1.0""#);
    give("null", "1.1 | tojson", r#""1.1""#);
    give("null", "0.0 / 0.0 | tojson", r#""NaN""#);
    give("null", "1.0 / 0.0 | tojson", r#""Infinity""#);
    give("null", "-1.0 / 0.0 | tojson", r#""-Infinity""#);
    give("null", "0 / 0 | tojson", r#""NaN""#);
    give("null", "1 / 0 | tojson", r#""Infinity""#);
    give("null", "-1 / 0 | tojson", r#""-Infinity""#);
    give("null", "try (3 % 0) catch .", r#""cannot calculate 3 % 0""#);
    give(
        "null",
        "try (-2 % 0) catch .",
        r#""cannot calculate -2 % 0""#,
    );
    give("null", "-2 % -2", "0");
    give("null", "-2 % -1", "0");
    give("null", "-2 % 2.1", "-2.0");
    give("null", "-2 % 3", "-2");
    give("null", "-2 % 2000000001", "-2");
    give("null", "-1 % -2", "-1");
    give("null", "-1 % -1", "0");
    give(
        "null",
        "try (-1 % 0) catch .",
        r#""cannot calculate -1 % 0""#,
    );
    give("null", "-1 % 2.1", "-1.0");
    give("null", "-1 % 3", "-1");
    give("null", "-1 % 2000000001", "-1");
    give("null", "0 % -2", "0");
    give("null", "0 % -1", "0");
    give("null", "try (0 % 0) catch .", r#""cannot calculate 0 % 0""#);
    give("null", "0 % 2.1", "0.0");
    give("null", "0 % 3", "0");
    give("null", "0 % 2000000001", "0");
    give("null", "2.1 % -2 | . * 1000 | round", "100");
    give("null", "2.1 % -1 | . * 1000 | round", "100");
    // TODO: jaq-json returns true here, but QueryValue does not yet make
    // isnan recognize the NaN value produced by float modulo.
    // give("null", "2.1 % 0 | isnan", "true");
    give("null", "2.1 % 2.1", "0.0");
    give("null", "2.1 % 3", "2.1");
    give("null", "2.1 % 2000000001", "2.1");
    give("null", "3 % -2", "1");
    give("null", "3 % -1", "0");
    give("null", "2.1 % 0 | tojson", r#""NaN""#);
    give("null", "3 % 2.1 | . * 1000 | round", "900");
    give("null", "3 % 3", "0");
    give("null", "3 % 2000000001", "3");
    give("null", "2000000001 % -2", "1");
    give("null", "2000000001 % -1", "0");
    give(
        "null",
        "try (2000000001 % 0) catch .",
        r#""cannot calculate 2000000001 % 0""#,
    );
    give("null", "2000000001 % 2.1 | . * 1000 | round", "1800");
    give("null", "2000000001 % 3", "0");
    give("null", "2000000001 % 2000000001", "0");

    give("1.0", "tonumber", "1.0");
    give(r#""1.0""#, "tonumber", "1.0");
    give(r#""42""#, "tonumber", "42");
    give(r#""null""#, "try tonumber catch -7", "-7");
    give(r#""true""#, "try tonumber catch -7", "-7");
    give(r#""str""#, "try tonumber catch -7", "-7");
    give(r#""\"str\"""#, "try tonumber catch -7", "-7");
    give(r#""[3, 4]""#, "try tonumber catch -7", "-7");
    give(r#""{\"a\": 1}""#, "try tonumber catch -7", "-7");

    give("false", "toboolean", "false");
    give(r#""true""#, "toboolean", "true");
    give(r#""false""#, "toboolean", "false");
    give(r#""null""#, "try toboolean catch -7", "-7");
    give(r#""3""#, "try toboolean catch -7", "-7");
    give(r#""str""#, "try toboolean catch -7", "-7");
    give(r#""\"str\"""#, "try toboolean catch -7", "-7");
    give(r#""[3, 4]""#, "try toboolean catch -7", "-7");
    give(r#""{\"a\": 1}""#, "try toboolean catch -7", "-7");

    give(
        "null",
        r#"("%FF" | @urid) == ([255] | tobytes | tostring)"#,
        "true",
    );
}

#[test]
fn jaq_nested_log_field_access() {
    give(
        r#"{"payload":{"features":{"category":"standard"}}}"#,
        ".payload.features.category",
        r#""standard""#,
    );
    give(
        r#"{
            "payload": {
                "features": {
                    "entity_id": 1001,
                    "score_vector": [0.413427, 0.750273, -0.008018],
                    "candidate": {
                        "item_id": 8194215875,
                        "score": 0.791828,
                        "labels": ["label_alpha"]
                    },
                    "category": "standard",
                    "request": {
                        "item_id": 7701052873,
                        "score": 0.093594,
                        "labels": ["label_beta", "label_gamma"]
                    }
                }
            }
        }"#,
        ".payload.features.category",
        r#""standard""#,
    );
    give("{}", ".payload.features.category", "null");
    give(r#"{"payload": null}"#, ".payload.features.category", "null");
    fail(
        "{}",
        ".payload.features.category[]",
        "cannot use null as iterable (array or object)",
    );
}

// Ported from jaq/jaq-json/tests/defs.rs.
#[test]
fn jaq_json_defs_compat() {
    give(
        r#"{"a": 1, "b": 2}"#,
        "to_entries",
        r#"[{"key":"a","value":1},{"key":"b","value":2}]"#,
    );
    give(
        r#"[{"key":"a","value":1},{"key":"b","value":2}]"#,
        "from_entries",
        r#"{"a":1,"b":2}"#,
    );
    give(
        r#"{"a": 1, "b": 2}"#,
        r#"with_entries(.key += "k")"#,
        r#"{"ak":1,"bk":2}"#,
    );
    give(
        "[null, 0]",
        "to_entries",
        r#"[{"key":0,"value":null},{"key":1,"value":0}]"#,
    );
    give("[]", "from_entries", "{}");

    give(
        r#"["foo", "bar"]"#,
        r#"map(in({"foo": 42}))"#,
        "[true,false]",
    );
    give("[2, 0]", "map(in([0,1]))", "[false,true]");
    give(r#""bar""#, r#"inside("foobar")"#, "true");
    give(
        r#"["baz", "bar"]"#,
        r#"inside(["foobar", "foobaz", "blarp"])"#,
        "true",
    );
    give(
        r#"["bazzzz", "bar"]"#,
        r#"inside(["foobar", "foobaz", "blarp"])"#,
        "false",
    );
    give(
        r#"{"foo": 12, "bar": [{"barp": 12}]}"#,
        r#"inside({"foo": 12, "bar":[1,2,{"barp":12, "blip":13}]})"#,
        "true",
    );
    give(
        r#"{"foo": 12, "bar": [{"barp": 15}]}"#,
        r#"inside({"foo": 12, "bar":[1,2,{"barp":12, "blip":13}]})"#,
        "false",
    );

    give(
        r#"{"foo":null,"abc":null,"fax":null,"az":null}"#,
        "keys",
        r#"["abc","az","fax","foo"]"#,
    );
    give("1", "[paths]", "[]");
    give("null", "[paths]", "[]");
    give("[1, 2]", "[paths]", "[[0],[1]]");
    give(
        r#"{"a": [1, [2]], "b": {"c": 3}}"#,
        "[paths]",
        r#"[["a"],["a",0],["a",1],["a",1,0],["b"],["b","c"]]"#,
    );
    give(
        "null",
        "[{a: 1, b: [2, 3]} | paths(. < [])]",
        r#"[["a"],["b",0],["b",1]]"#,
    );
    give(
        "null",
        r#"def paths:
  { x: ., p: [] } |
  recurse((.x | keys_unsorted?)[] as $k | .x |= .[$k] | .p += [$k]) |
  .p | if . == [] then empty else . end;
{a: [1, [2]], b: {c: 3}} | [paths]"#,
        r#"[["a"],["a",0],["a",1],["a",1,0],["b"],["b","c"]]"#,
    );

    give("[[1, 3], [2]]", "transpose", "[[1,2],[3,null]]");
    give("[[1, 3], [2, 4]]", "transpose", "[[1,2],[3,4]]");
    give(
        "[[4, 1, 7], [8, 5, 2], [3, 6, 9]]",
        r#"walk(if . < [] then . else sort end)"#,
        "[[1,4,7],[2,5,8],[3,6,9]]",
    );
    give(
        r#"{"a": {"b": 1, "c": 2}}"#,
        r#"walk(if . < {} then . + 1 else . + {"l": length} end)"#,
        r#"{"a":{"b":2,"c":3,"l":2},"l":1}"#,
    );
    give("1", "[while(. < 100; . * 2)]", "[1,2,4,8,16,32,64]");
    give(
        r#""a""#,
        r#"[while(length < 4; . + "a")]"#,
        r#"["a","aa","aaa"]"#,
    );
    give(
        "[1, 2, 3]",
        "[while(length > 0; .[1:])]",
        "[[1,2,3],[2,3],[3]]",
    );
    give("50", "until(. > 100; . * 2)", "200");
    give("[1, 2, 3]", "until(length == 1; .[1:]) | .[0]", "3");
    give(
        "5",
        "[.,1] | until(.[0] < 1; [.[0] - 1, .[1] * .[0]]) | .[1]",
        "120",
    );
    give(
        "null",
        r#"[0, 0 == 0, {}.a, "hello", {}, [] | @json]"#,
        r#"["0","true","null","\"hello\"","{}","[]"]"#,
    );
}

// Selected backend-sensitive cases from jaq/jaq-core/tests/funs.rs.
#[test]
fn jaq_core_funs_backend_compat() {
    give("[0, null, \"a\"]", "keys_unsorted", "[0,1,2]");
    give(r#"{"a": 1, "b": 2}"#, "keys_unsorted", r#"["a","b"]"#);

    give("null", "[range(0; 6;  2)]", "[0,2,4]");
    give("null", "[range(0; 6; -2)]", "[]");
    give("null", "[range(0; -6; 2)]", "[]");
    give("null", "[range(0; -6; -2)]", "[0,-2,-4]");
    give("null", "[range(0; 0; 0)]", "[]");
    give("null", "[range(0.0; 2; 0.5)]", "[0.0,0.5,1.0,1.5]");
    give("null", "[limit(3; range(0; 1/0; 1))]", "[0,1,2]");
    give("null", "[limit(3; range(0; -1/0; -1))]", "[0,-1,-2]");
    give("null", "[limit(3; range(0; 6; 0))]", "[0,0,0]");
    give("null", "[limit(3; range(0; -6; 0))]", "[0,0,0]");

    give(
        "null",
        "[{a: 1, b: [2, 3]} | skip(1; path_value(..))]",
        r#"[[["a"],1],[["b"],[2,3]],[["b",0],2],[["b",1],3]]"#,
    );
}

// Selected backend-sensitive cases from jaq/jaq-std/tests/funs.rs.
#[test]
fn jaq_std_funs_backend_compat() {
    give(r#""aAaAäの""#, "ascii_upcase", r#""AAAAäの""#);
    give(r#""aAaAäの""#, "ascii_downcase", r#""aaaaäの""#);

    give(r#""❤ の""#, "explode", "[10084,32,12398]");
    give(r#""y̆""#, "explode", "[121,774]");
    give(r#""❤ の""#, "explode | implode", r#""❤ の""#);
    give(r#""y̆""#, "explode | implode", r#""y̆""#);
    give("null", "[1114112] | try implode catch -1", "-1");

    give(
        r#""hello cruel world""#,
        "encode_base64",
        r#""aGVsbG8gY3J1ZWwgd29ybGQ=""#,
    );
    give(
        r#""hello cruel world""#,
        "encode_base64 | decode_base64",
        r#""hello cruel world""#,
    );
    give(
        r#""<p style='visibility: hidden'>sneaky</p>""#,
        "escape_html",
        r#""&lt;p style=&apos;visibility: hidden&apos;&gt;sneaky&lt;/p&gt;""#,
    );
    give(
        r#""abc123 ?#+&[]""#,
        "encode_uri",
        r#""abc123%20%3F%23%2B%26%5B%5D""#,
    );

    give("[]", "group_by(.)", "[]");
    give(
        r#"[{"key":1, "value": "foo"},{"key":2, "value":"bar"},{"key":1,"value":"baz"}]"#,
        "group_by(.key)",
        r#"[[{"key":1,"value":"foo"},{"key":1,"value":"baz"}],[{"key":2,"value":"bar"}]]"#,
    );

    give(r#""foo""#, "utf8bytelength", "3");
    give(r#""ƒoo""#, "utf8bytelength", "4");
    give(r#""नमस्ते""#, "utf8bytelength", "18");

    give(
        "null",
        "[-2.2, -1.1, 0, 1.1, 2.2 | sin as $s | cos as $c | $s * $s + $c * $c]",
        "[1.0,1.0,1.0,1.0,1.0]",
    );
    give(
        "null",
        "[3, 3.25, 3.5 | modf]",
        "[[0.0,3.0],[0.25,3.0],[0.5,3.0]]",
    );
    give(
        "null",
        "[pow(0.25, 4, 9; 1, 0.5, 2)]",
        "[0.25,0.5,0.0625,4.0,2.0,16.0,9.0,3.0,81.0]",
    );
    give(
        "null",
        "[fma(2, 1; 3, 4; 4, 5)]",
        "[10.0,11.0,12.0,13.0,7.0,8.0,8.0,9.0]",
    );

    give("null", "[0, 1][1 | round]", "1");
    give("null", " 1   | round", "1");
    // TODO: jaq-std returns integer 1 here, but QueryValue currently
    // preserves the float representation as 1.0.
    // give("null", " 1.0 | round", "1");
    give("null", "-1   | round", "-1");
    // TODO: jaq-std integerizes these rounded float results, while QueryValue
    // currently preserves them as float values such as -1.0 and -2.0.
    // give("null", "-1.0 | round", "-1");
    // give("null", "-1.5 | round", "-2");
    // give("null", "-1.5 | floor", "-2");
    // give("null", "-1.5 | ceil ", "-1");
    // give("null", "-1.4 | round", "-1");
    // give("null", "-1.4 | floor", "-2");
    // give("null", "-1.4 | ceil ", "-1");
    give(
        "null",
        "2e22 | round | tostring",
        r#""20000000000000000000000""#,
    );

    give(r#""foobar""#, r#"startswith("")"#, "true");
    give(r#""foobar""#, r#"startswith("bar")"#, "false");
    give(r#""foobar""#, r#"startswith("foo")"#, "true");
    give(r#""""#, r#"startswith("foo")"#, "false");

    give(r#""foobar""#, r#"endswith("")"#, "true");
    give(r#""foobar""#, r#"endswith("foo")"#, "false");
    give(r#""foobar""#, r#"endswith("bar")"#, "true");
    give(r#""""#, r#"endswith("foo")"#, "false");

    give(r#""foobar""#, r#"ltrimstr("")"#, r#""foobar""#);
    give(r#""foobar""#, r#"ltrimstr("foo")"#, r#""bar""#);
    give(r#""foobar""#, r#"ltrimstr("bar")"#, r#""foobar""#);
    give(r#""اَلْعَرَبِيَّةُ""#, r#"ltrimstr("ا")"#, r#""َلْعَرَبِيَّةُ""#);

    give(r#""foobar""#, r#"rtrimstr("")"#, r#""foobar""#);
    give(r#""foobar""#, r#"rtrimstr("bar")"#, r#""foo""#);
    give(r#""foobar""#, r#"rtrimstr("foo")"#, r#""foobar""#);
    give(r#""اَلْعَرَبِيَّةُ""#, r#"rtrimstr("ا")"#, r#""اَلْعَرَبِيَّةُ""#);

    give(r#""""#, "trim", r#""""#);
    give(r#"" ""#, "trim", r#""""#);
    give(r#""foo""#, "trim", r#""foo""#);
    give(r#"" foo  ""#, "trim", r#""foo""#);
    give(r#"" اَلْعَرَبِيَّةُ ""#, "trim", r#""اَلْعَرَبِيَّةُ""#);

    give(r#""""#, "ltrim", r#""""#);
    give(r#"" ""#, "ltrim", r#""""#);
    give(r#"" foo  ""#, "ltrim", r#""foo  ""#);
    give(r#"" اَلْعَرَبِيَّةُ ""#, "ltrim", r#""اَلْعَرَبِيَّةُ ""#);

    give(r#""""#, "rtrim", r#""""#);
    give(r#"" ""#, "rtrim", r#""""#);
    give(r#""  foo ""#, "rtrim", r#""  foo""#);
    give(r#"" اَلْعَرَبِيَّةُ ""#, "rtrim", r#"" اَلْعَرَبِيَّةُ""#);
}

// Ported from jaq-core/tests/path.rs.
#[test]
fn jaq_core_path_compat() {
    give("[0, 1, 2]", ".[-4]", "null");
    give("[0, 1, 2]", ".[-3]", "0");
    give("[0, 1, 2]", ".[-1]", "2");
    give("[0, 1, 2]", ".[0]", "0");
    give("[0, 1, 2]", ".[2]", "2");
    give("[0, 1, 2]", ".[3]", "null");
    give(r#"{"a": 1}"#, ".a", "1");
    give(r#"{"a": 1}"#, ".a?", "1");
    give(r#"{"a": 1}"#, r#"."a""#, "1");
    give(r#"{"a": 1}"#, r#".["a"]"#, "1");
    give(r#"{"a_": 1}"#, ".a_", "1");
    give(r#"{"_a": 1}"#, "._a", "1");
    give(r#"{"_0": 1}"#, "._0", "1");
    give("[0, 1, 2]", r#".["a", 0, 0 == 0]?"#, "0");
    give("[0, 1, 2]", r#".[3]?"#, "null");
    gives(r#""asdf""#, ".[0]?", &[]);
    give("1", "[1, 2, 3][.]", "2");
    gives(r#"{"a": 1, "b": 2}"#, r#".["b", "a"]"#, &["2", "1"]);
    give("[0, 1, 2, 1, 2, 3]", ".[[1, 2]]", "[1,3]");
    give(r#"{"a": [1, 2, 1, 2], "b": [1, 2]}"#, ".a[.b]", "[0,2]");
    give(
        r#"{"a": [1, 2, 1, 2], "b": [1, 2]}"#,
        "(.a + [3])[.b]",
        "[0,2]",
    );
    give("[0, 1]", ".[[]]", "[]");
    give("[0, 1, 2, 3]", r#".[{"start":1,"end":3}]"#, "[1,2]");
    give(
        r#"{"a": [0, 1, 2, 3], "r": {"start": 1, "end": 3}}"#,
        ".a[.r]",
        "[1,2]",
    );
    give(r#""abcd""#, r#".[{"start":1,"end":3}]"#, r#""bc""#);
    give(
        "null",
        r#"[[65], "B", 67, 68] | tobytes | .[{"start":1,"end":3}] | tostring"#,
        r#""BC""#,
    );

    gives("[0, 1, 2]", ".[]", &["0", "1", "2"]);
    gives(r#"{"a": [1, 2]}"#, ".a[]", &["1", "2"]);
    gives(r#"{"a": 1, "b": 2}"#, ".[]", &["1", "2"]);
    gives(r#""asdf""#, ".[]?", &[]);

    give(r#""Möwe""#, ".[1:-1]", r#""öw""#);
    give(r#""नमस्ते""#, ".[1:5]", r#""मस्त""#);
    give("[0, 1, 2]", ".[-4:4]", "[0,1,2]");
    give("[0, 1, 2]", ".[0:3]", "[0,1,2]");
    give("[0, 1, 2]", ".[1:]", "[1,2]");
    give("[0, 1, 2]", ".[:-1]", "[0,1]");
    give("[0, 1, 2]", ".[1:0]", "[]");
    give("[0, 1, 2]", ".[4:5]", "[]");
    give("[0, 1, 2]", ".[0:2,3.14]?", "[0,1]");

    give("[1, 2]", ".[] = .", "[[1,2],[1,2]]");
    give(
        r#"{"a": [1,2], "b": 3}"#,
        ".a[] = .b+.b",
        r#"{"a":[6,6],"b":3}"#,
    );
    give("{}", ".a  |= .+1", r#"{"a":1}"#);
    give("{}", ".a? |= .+1", r#"{"a":1}"#);
    give("null", ".a", "null");
    give("null", ".[0]", "null");
    give("null", "try .[0[]]? catch 1", "1");
    give("null", "1, (.[0[]])?", "1");

    give(r#"{"a": 1}"#, ".b |= .", r#"{"a":1,"b":null}"#);
    give(r#"{"a": 1}"#, ".b |= 1", r#"{"a":1,"b":1}"#);
    give(r#"{"a": 1}"#, ".b |= .+1", r#"{"a":1,"b":1}"#);
    give(r#"{"a": 1, "b": 2}"#, ".b |= {}[]", r#"{"a":1}"#);
    give(r#"{"a": 1, "b": 2}"#, ".a += 1", r#"{"a":2,"b":2}"#);
    give("[0, 1, 2]", ".[1] |= .+2", "[0,3,2]");
    give("[0, 1, 2]", ".[-1,-1] |= {}[]", "[0]");
    give("[0, 1, 2]", ".[0, 0] |= {}[]", "[2]");
    give("[0, 1, 2]", r#".["a", 0]? |= .+1"#, "[1,1,2]");
    give("[0, 1, 2]", r#".[3]? |= .+1"#, "[0,1,2]");
    give(r#""asdf""#, ".[0]? |= .+1", r#""asdf""#);

    give("[]", ".[] |= . or 0", "[]");
    gives("[]", ".[] |= .,.", &["[]", "[]"]);
    give("[]", ".[] |= (.,.)", "[]");
    give("[0]", ".[] |= .+1 | .+[2]", "[1,2]");
    give("[[1]]", ".[] |= .[] |= .+1", "[[2]]");
    give("[[1]]", ".[] |= .[] += 1", "[[2]]");
    give("[1]", ".[] |= .+1", "[2]");
    give("[[1]]", ".[][] |= .+1", "[[2]]");
    give(
        r#"{"a": 1, "b": 2}"#,
        r#".[] |= ((if .>1 then . else {}[] end) | .+1)"#,
        r#"{"b":3}"#,
    );
    give(r#"[[0, 1], "a"]"#, ".[][]? |= .+1", r#"[[1,2],"a"]"#);

    give("[0, 1, 2]", ".[:2] |= [.[] | .+5]", "[5,6,2]");
    give("[0, 1, 2]", ".[-2:-1] |= [5]+.", "[0,5,1,2]");
    give("[0, 1, 2]", ".[-2:-1,-1] |= [5,6]+.", "[0,5,6,5,6,1,2]");
    give("[0, 1, 2]", ".[:2,3.0]? |= [.[] | .+1]", "[1,2,2]");
    give(
        r#"{"a": [0, 1, 2, 3], "r": {"start": 1, "end": 3}}"#,
        ".a[.r] |= [9]",
        r#"{"a":[0,9,3],"r":{"end":3,"start":1}}"#,
    );

    give(r#"{"a": 1}"#, ".a |= (.,.+1)", r#"{"a":1}"#);
    gives("1", ". |= {}[]", &[]);
    gives("1", ". |= (.,.)", &["1", "1"]);
    give("[1]", ".[] |= (., .+1)", "[1,2]");
    give("[1, 3]", ".[] |= (., .+1)", "[1,2,3,4]");
    give("[1, 2, 3, 4, 5]", ".[] |= {}[]", "[]");
    give("[1, 2]", ".[] // . |= 3", "[3,3]");
    give("[]", ".[] // . |= 3", "3");
}
