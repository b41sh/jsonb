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

use std::io::Write;

use goldenfile::Mint;
use jsonb::jsonpath::parse_json_path;

#[test]
fn test_json_path() {
    let mut mint = Mint::new("tests/it/testdata");
    let mut file = mint.new_goldenfile("json_path.txt").unwrap();
    let cases = &[
        r#"$"#,
        r#"$.*"#,
        r#"$[*]"#,
        r#"$.store.book[*].*"#,
        r#"$.store.book[0].price"#,
        r#"$.store.book[last].isbn"#,
        r#"$.store.book[0,1, last - 2].price"#,
        r#"$.store.book[0,1 to last-1]"#,
        r#"$.store.book?(@.isbn).price"#,
        r#"$.store.book?(@.price > 10).title"#,
        r#"$.store.book?(@.price < $.expensive).price"#,
        r#"$."store":book["price"]"#,
        r#"$.store.book?(@.price < 10 && @.category == "fiction")"#,
        r#"$.store.book?(@.price > 10 || @.category == "reference")"#,
        r#"$.store.book?(@.price > 20 && (@.category == "reference" || @.category == "fiction"))"#,
    ];

    for case in cases {
        println!("\n---case={:?}", case);
        //let json_path = parse_json_path(case.as_bytes()).unwrap();
        let json_path = parse_json_path(case.as_bytes());
        println!("json_path={:?}", json_path);
        let json_path = json_path.unwrap();

        writeln!(file, "---------- Input ----------").unwrap();
        writeln!(file, "{}", case).unwrap();
        writeln!(file, "---------- Output ---------").unwrap();
        writeln!(file, "{}", json_path).unwrap();
        writeln!(file, "---------- AST ------------").unwrap();
        writeln!(file, "{:#?}", json_path).unwrap();
        writeln!(file, "\n").unwrap();
    }
}

#[test]
fn test_json_path_error() {
    let cases = &[
        r#"x"#,
        r#"$.["#,
        r#"$X"#,
        r#"$."#,
        r#"$.prop."#,
        r#"$.."#,
        r#"$.prop.."#,
        r#"$.foo bar"#,
        r#"$[0, 1, 2 4]"#,
        r#"$['1','2',]"#,
        r#"$['1', ,'3']"#,
        r#"$['aaa'}'bbb']"#,
    ];

    for case in cases {
        let res = parse_json_path(case.as_bytes());
        assert!(res.is_err());
    }
}
