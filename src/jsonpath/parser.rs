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

use nom::{
    branch::alt,
    bytes::complete::{escaped, tag, tag_no_case},
    character::complete::{alphanumeric1, char, i32, i64, multispace0, one_of, u32, u64},
    combinator::{map, opt, value},
    multi::{many1, separated_list1},
    number::complete::double,
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};

use crate::error::Error;
use crate::jsonpath::path::*;
use std::borrow::Cow;

/// Parsing the input string to JSON Path.
pub fn parse_json_path<'a>(input: &'a [u8]) -> Result<JsonPath<'a>, Error> {
    match json_path(input) {
        Ok((rest, json_path)) => {
            if !rest.is_empty() {
                return Err(Error::InvalidJsonPath);
            }
            Ok(json_path)
        }
        Err(nom::Err::Error(_err) | nom::Err::Failure(_err)) => Err(Error::InvalidJsonb),
        Err(nom::Err::Incomplete(_)) => unreachable!(),
    }
}

fn json_path<'a>(input: &'a [u8]) -> IResult<&'a [u8], JsonPath<'a>> {
    map(delimited(multispace0, many1(path), multispace0), |paths| {
        JsonPath { paths }
    })(input)
}

fn raw_string<'a>(input: &'a [u8]) -> IResult<&'a [u8], &'a [u8]> {
    escaped(alphanumeric1, '\\', one_of("\"n\\"))(input)
}

fn string<'a>(input: &'a [u8]) -> IResult<&'a [u8], &'a [u8]> {
    alt((
        delimited(char('\''), raw_string, char('\'')),
        delimited(char('"'), raw_string, char('"')),
    ))(input)
}

fn bracket_wildcard<'a>(input: &'a [u8]) -> IResult<&'a [u8], ()> {
    value(
        (),
        delimited(
            char('['),
            delimited(multispace0, char('*'), multispace0),
            char(']'),
        ),
    )(input)
}

fn colon_field<'a>(input: &'a [u8]) -> IResult<&'a [u8], &'a [u8]> {
    preceded(char(':'), alphanumeric1)(input)
}

fn dot_field<'a>(input: &'a [u8]) -> IResult<&'a [u8], &'a [u8]> {
    preceded(char('.'), alphanumeric1)(input)
}

fn descent_field<'a>(input: &'a [u8]) -> IResult<&'a [u8], &'a [u8]> {
    preceded(tag(".."), alphanumeric1)(input)
}

fn array_index<'a>(input: &'a [u8]) -> IResult<&'a [u8], i32> {
    delimited(
        terminated(char('['), multispace0),
        i32,
        preceded(multispace0, char(']')),
    )(input)
}

fn array_indices<'a>(input: &'a [u8]) -> IResult<&'a [u8], Vec<i32>> {
    delimited(
        terminated(char('['), multispace0),
        separated_list1(delimited(multispace0, char(','), multispace0), i32),
        preceded(multispace0, char(']')),
    )(input)
}

fn object_field<'a>(input: &'a [u8]) -> IResult<&'a [u8], &'a [u8]> {
    delimited(
        terminated(char('['), multispace0),
        string,
        preceded(multispace0, char(']')),
    )(input)
}

fn object_fields<'a>(input: &'a [u8]) -> IResult<&'a [u8], Vec<&'a [u8]>> {
    delimited(
        terminated(char('['), multispace0),
        separated_list1(delimited(multispace0, char(','), multispace0), string),
        preceded(multispace0, char(']')),
    )(input)
}

fn array_slice<'a>(input: &'a [u8]) -> IResult<&'a [u8], Path<'a>> {
    map(
        delimited(
            char('['),
            tuple((
                delimited(multispace0, opt(i32), multispace0),
                char(':'),
                delimited(multispace0, opt(i32), multispace0),
                opt(preceded(
                    char(':'),
                    delimited(multispace0, u32, multispace0),
                )),
            )),
            char(']'),
        ),
        |(opt_start, _, opt_end, opt_step)| Path::ArraySlice {
            start: opt_start,
            end: opt_end,
            step: opt_step,
        },
    )(input)
}

fn path<'a>(input: &'a [u8]) -> IResult<&'a [u8], Path<'a>> {
    alt((
        value(Path::Root, char('$')),
        value(Path::Current, char('@')),
        value(Path::DotWildcard, tag(".*")),
        value(Path::DescentWildcard, tag("..*")),
        value(Path::BracketWildcard, bracket_wildcard),
        map(colon_field, |v| {
            Path::ColonField(Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(v) }))
        }),
        map(dot_field, |v| {
            Path::DotField(Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(v) }))
        }),
        map(descent_field, |v| {
            Path::DescentField(Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(v) }))
        }),
        map(array_index, Path::ArrayIndex),
        map(array_indices, Path::ArrayIndices),
        map(object_field, |v| {
            Path::ObjectField(Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(v) }))
        }),
        map(object_fields, |v| {
            let fields = v
                .iter()
                .map(|s| Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(s) }))
                .collect();
            Path::ObjectFields(fields)
        }),
        map(array_slice, |v| v),
        map(filter_expr, |v| Path::FilterExpr(Box::new(v))),
    ))(input)
}

fn filter_expr<'a>(input: &'a [u8]) -> IResult<&'a [u8], Expr<'a>> {
    map(
        delimited(
            tag("[?("),
            delimited(multispace0, expr, multispace0),
            tag(")]"),
        ),
        |v| v,
    )(input)
}

fn paths<'a>(input: &'a [u8]) -> IResult<&'a [u8], Vec<Path<'a>>> {
    many1(path)(input)
}

fn op<'a>(input: &'a [u8]) -> IResult<&'a [u8], BinaryOperator> {
    alt((
        value(BinaryOperator::Eq, tag("==")),
        value(BinaryOperator::NotEq, tag("!=")),
        value(BinaryOperator::Lt, tag("<")),
        value(BinaryOperator::Lte, tag("<=")),
        value(BinaryOperator::Gt, tag(">")),
        value(BinaryOperator::Gte, tag(">=")),
        value(BinaryOperator::Match, tag("=~")),
        value(BinaryOperator::In, tag_no_case("in")),
        value(BinaryOperator::Nin, tag_no_case("nin")),
        value(BinaryOperator::Subsetof, tag_no_case("subsetof")),
        value(BinaryOperator::Anyof, tag_no_case("anyof")),
        value(BinaryOperator::Noneof, tag_no_case("noneof")),
        value(BinaryOperator::Size, tag_no_case("size")),
        value(BinaryOperator::Empty, tag_no_case("empty")),
    ))(input)
}

fn path_value<'a>(input: &'a [u8]) -> IResult<&'a [u8], PathValue<'a>> {
    alt((
        value(PathValue::Null, tag("null")),
        value(PathValue::Boolean(true), tag("true")),
        value(PathValue::Boolean(false), tag("false")),
        map(u64, PathValue::UInt64),
        map(i64, PathValue::Int64),
        map(double, PathValue::Float64),
        map(string, |v| {
            PathValue::String(Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(v) }))
        }),
    ))(input)
}

fn sub_expr<'a>(input: &'a [u8]) -> IResult<&'a [u8], Expr<'a>> {
    alt((
        map(paths, Expr::Paths),
        map(path_value, |v| Expr::Value(Box::new(v))),
    ))(input)
}

fn expr<'a>(input: &'a [u8]) -> IResult<&'a [u8], Expr<'a>> {
    // TODO, support more complex expressions.
    alt((
        map(
            tuple((
                delimited(multispace0, sub_expr, multispace0),
                op,
                delimited(multispace0, sub_expr, multispace0),
            )),
            |(left, op, right)| Expr::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
        ),
        map(sub_expr, |v| v),
    ))(input)
}
