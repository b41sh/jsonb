use jaq_core::load::Arena;
use jaq_core::load::File;
use jaq_core::load::Loader;
use jaq_core::unwrap_valr;
use jaq_core::Compiler;
use jaq_core::Ctx;
use jaq_core::Vars;

use crate::core::QueryValue;
use crate::jaq::defs;
use crate::jaq::funs;
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

    let input = QueryValue::from_owned(input.parse::<OwnedJsonb>().unwrap());
    let ctx = Ctx::<JsonbData>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input))
        .map(unwrap_valr)
        .map(|value| value.unwrap().to_string())
        .collect()
}

fn give(input: &str, filter: &'static str, output: &str) {
    gives(input, filter, &[output]);
}

fn gives(input: &str, filter: &'static str, outputs: &[&str]) {
    assert_eq!(
        run_filter(filter, input),
        outputs,
        "input: {input}, filter: {filter}"
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
}

// Ported from jaq/jaq-json/tests/defs.rs.
#[test]
fn jaq_json_defs_compat() {
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

    give(r#"{"a": 1}"#, ".a |= (.,.+1)", r#"{"a":1}"#);
    gives("1", ". |= {}[]", &[]);
    gives("1", ". |= (.,.)", &["1", "1"]);
    give("[1]", ".[] |= (., .+1)", "[1,2]");
    give("[1, 3]", ".[] |= (., .+1)", "[1,2,3,4]");
    give("[1, 2, 3, 4, 5]", ".[] |= {}[]", "[]");
    give("[1, 2]", ".[] // . |= 3", "[3,3]");
    give("[]", ".[] // . |= 3", "3");
}
