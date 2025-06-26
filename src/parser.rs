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

use super::constants::*;
use super::error::Error;
use super::error::ParseErrorCode;
use super::error::Result;
use super::number::Number;
use super::util::parse_string;
use super::value::Object;
use super::value::Value;
use crate::core::Decoder;

use std::str::FromStr;

use crate::Decimal128;
use crate::Decimal256;
use crate::Decimal64;
use ethnum::i256;

const MAX_DECIMAL64_PRECISION: usize = 18;
const MAX_DECIMAL128_PRECISION: usize = 38;
const MAX_DECIMAL256_PRECISION: usize = 76;

const UINT64_MIN: i128 = 0i128;
const UINT64_MAX: i128 = 18_446_744_073_709_551_615i128;
const INT64_MIN: i128 = -9_223_372_036_854_775_808i128;
const INT64_MAX: i128 = 9_223_372_036_854_775_807i128;
const DECIMAL64_MIN: i128 = -999999999999999999i128;
const DECIMAL64_MAX: i128 = 999999999999999999i128;
const DECIMAL128_MIN: i128 = -99999999999999999999999999999999999999i128;
const DECIMAL128_MAX: i128 = 99999999999999999999999999999999999999i128;

/// The binary `JSONB` contains three parts, `Header`, `JEntry` and `RawData`.
/// This structure can be nested. Each group of structures starts with a `Header`.
/// The upper-level `Value` will store the `Header` length or offset of
/// the lower-level `Value`.
///
/// `Header` stores the type of the `Value`, include `Array`, `Object` and `Scalar`,
/// `Scalar` has only one `Value`, and a corresponding `JEntry`.
/// `Array` and `Object` are nested type, they have multiple lower-level `Values`.
/// So the `Header` also stores the number of lower-level `Values`.
///
/// `JEntry` stores the types of `Scalar Value`, including `Null`, `True`, `False`,
/// `Number`, `String` and `Container`. They have three different decode methods.
/// 1. `Null`, `True` and `False` can be obtained by `JEntry`, no extra work required.
/// 2. `Number` and `String` has related `RawData`, `JEntry` store the length
///    or offset of this data, the `Value` can be read out and then decoded.
/// 3. `Container` is actually a nested `Array` or `Object` with the same structure,
///    `JEntry` store the length or offset of the lower-level `Header`,
///    from where the same decode process can begin.
///
///    `RawData` is the encoded `Value`.
///    `Number` is a variable-length `Decimal`, store both int and float value.
///    `String` is the original string, can be borrowed directly without extra decode.
///    `Array` and `Object` is a lower-level encoded `JSONB` value.
///    The upper-level doesn't care about the specific content.
///    Decode can be executed recursively.
///
///    Decode `JSONB` Value from binary bytes.
pub fn from_slice(buf: &[u8]) -> Result<Value<'_>> {
    let mut decoder = Decoder::new(buf);
    match decoder.decode() {
        Ok(value) => Ok(value),
        // for compatible with the first version of `JSON` text, parse it again
        Err(_) => parse_value(buf),
    }
}

// Parse JSON text to JSONB Value.
// Inspired by `https://github.com/jorgecarleitao/json-deserializer`
// Thanks Jorge Leitao.
pub fn parse_value(buf: &[u8]) -> Result<Value<'_>> {
    let mut parser = Parser::new(buf);
    parser.parse()
}

struct Parser<'a> {
    buf: &'a [u8],
    idx: usize,
}

impl<'a> Parser<'a> {
    fn new(buf: &'a [u8]) -> Parser<'a> {
        Self { buf, idx: 0 }
    }

    fn parse(&mut self) -> Result<Value<'a>> {
        let val = self.parse_json_value()?;
        self.skip_unused();
        if self.idx < self.buf.len() {
            self.step();
            return Err(self.error(ParseErrorCode::UnexpectedTrailingCharacters));
        }
        Ok(val)
    }

    fn parse_json_value(&mut self) -> Result<Value<'a>> {
        self.skip_unused();
        let c = self.next()?;
        match c {
            b'n' => self.parse_json_null(),
            b't' => self.parse_json_true(),
            b'f' => self.parse_json_false(),
            b'0'..=b'9' | b'-' | b'+' | b'.' => self.parse_json_number(),
            b'"' => self.parse_json_string(),
            b'[' => self.parse_json_array(),
            b'{' => self.parse_json_object(),
            _ => {
                self.step();
                Err(self.error(ParseErrorCode::ExpectedSomeValue))
            }
        }
    }

    #[inline]
    fn next(&mut self) -> Result<&u8> {
        match self.buf.get(self.idx) {
            Some(c) => Ok(c),
            None => Err(self.error(ParseErrorCode::InvalidEOF)),
        }
    }

    #[inline]
    fn must_is(&mut self, c: u8) -> Result<()> {
        match self.buf.get(self.idx) {
            Some(v) => {
                self.step();
                if v == &c {
                    Ok(())
                } else {
                    Err(self.error(ParseErrorCode::ExpectedSomeIdent))
                }
            }
            None => Err(self.error(ParseErrorCode::InvalidEOF)),
        }
    }

    #[inline]
    fn check_next(&mut self, c: u8) -> bool {
        if self.idx < self.buf.len() {
            let v = self.buf.get(self.idx).unwrap();
            if v == &c {
                return true;
            }
        }
        false
    }

    #[inline]
    fn check_next_either(&mut self, c1: u8, c2: u8) -> bool {
        if self.idx < self.buf.len() {
            let v = self.buf.get(self.idx).unwrap();
            if v == &c1 || v == &c2 {
                return true;
            }
        }
        false
    }

    #[inline]
    fn check_digit(&mut self) -> bool {
        if self.idx < self.buf.len() {
            let v = self.buf.get(self.idx).unwrap();
            if v.is_ascii_digit() {
                return true;
            }
        }
        false
    }

    #[inline]
    fn step_digits(&mut self) -> usize {
        let mut len = 0;
        while self.idx < self.buf.len() {
            let c = self.buf.get(self.idx).unwrap();
            if !c.is_ascii_digit() {
                break;
            }
            len += 1;
            self.step();
        }
        len
    }

    #[inline]
    fn step(&mut self) {
        self.idx += 1;
    }

    #[inline]
    fn step_by(&mut self, n: usize) {
        self.idx += n;
    }

    fn error(&self, code: ParseErrorCode) -> Error {
        let pos = self.idx;
        Error::Syntax(code, pos)
    }

    #[inline]
    fn skip_unused(&mut self) {
        while self.idx < self.buf.len() {
            let c = self.buf[self.idx];

            // Fast path: handle common whitespace characters
            if c.is_ascii_whitespace() {
                self.idx += 1;
                continue;
            }

            // Slow path: handle escape sequences
            if c == b'\\' && self.idx + 1 < self.buf.len() {
                let next_c = self.buf[self.idx + 1];

                // Handle simple escapes \n, \r, \t
                let simple_escape = matches!(next_c, b'n' | b'r' | b't');
                if simple_escape {
                    self.idx += 2;
                    continue;
                }

                // Handle \x0C escape
                let hex_escape = self.idx + 3 < self.buf.len()
                    && next_c == b'x'
                    && self.buf[self.idx + 2] == b'0'
                    && self.buf[self.idx + 3] == b'C';
                if hex_escape {
                    self.idx += 4;
                    continue;
                }
            }

            break;
        }
    }

    fn parse_json_null(&mut self) -> Result<Value<'a>> {
        let data = [b'n', b'u', b'l', b'l'];
        for v in data.into_iter() {
            self.must_is(v)?;
        }
        Ok(Value::Null)
    }

    fn parse_json_true(&mut self) -> Result<Value<'a>> {
        let data = [b't', b'r', b'u', b'e'];
        for v in data.into_iter() {
            self.must_is(v)?;
        }
        Ok(Value::Bool(true))
    }

    fn parse_json_false(&mut self) -> Result<Value<'a>> {
        let data = [b'f', b'a', b'l', b's', b'e'];
        for v in data.into_iter() {
            self.must_is(v)?;
        }
        Ok(Value::Bool(false))
    }

    /// Parse a JSON number using a single-pass approach with multiple fallback strategies.
    ///
    /// This function implements a high-performance JSON number parsing algorithm that:
    /// 1. First attempts to parse the number as an i128 (for Decimal128/Int64/UInt64)
    /// 2. Falls back to i256 (for Decimal256) if precision exceeds i128 capacity
    /// 3. Finally falls back to Float64 if all other methods fail
    ///
    /// The algorithm handles signs, leading zeros, decimal points, and exponents in a single pass,
    /// avoiding multiple traversals of the input for better performance. It uses unsafe operations
    /// for arithmetic to maximize speed, with appropriate overflow checks through precision limits.
    fn parse_json_number(&mut self) -> Result<Value<'a>> {
        let start_idx = self.idx;
        let mut negative = false;

        let c = self.next()?;
        if *c == b'-' {
            negative = true;
            self.step();
        } else if *c == b'+' {
            self.step();
        }

        loop {
            if self.check_next(b'0') {
                self.step();
            } else {
                break;
            }
        }

        let num_start_idx = self.idx;

        let mut value = 0_i128;
        let mut scale = 0_i32;
        let mut fraction_offset = None;
        let mut has_exponent = false;
        let mut precision = 0;
        while precision < MAX_DECIMAL128_PRECISION {
            if self.check_digit() {
                let digit = (self.buf[self.idx] - b'0') as i128;

                value = unsafe { value.unchecked_mul(10_i128) };
                value = unsafe { value.unchecked_add(digit) };
                self.step();
            } else if self.check_next(b'.') {
                if fraction_offset.is_some() {
                    return Err(self.error(ParseErrorCode::InvalidNumberValue));
                }
                fraction_offset = Some(self.idx);
                self.step();
                continue;
            } else {
                break;
            }
            precision += 1;
            if fraction_offset.is_some() {
                scale += 1;
            }
        }

        if precision == MAX_DECIMAL128_PRECISION {
            if fraction_offset.is_none() {
                let len = self.step_digits();
                precision += len;
                if self.check_next(b'.') {
                    fraction_offset = Some(self.idx);
                    self.step();
                }
            }
                if fraction_offset.is_some() {
                let len = self.step_digits();
                precision += len;
                scale += len as i32;
            }
        }

        if self.check_next_either(b'E', b'e') {
            has_exponent = true;
            self.step();
            if self.check_next_either(b'+', b'-') {
                self.step();
            }
            let len = self.step_digits();
            if len == 0 {
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }

        if !has_exponent && precision <= MAX_DECIMAL128_PRECISION {
            if negative {
                value = value.checked_neg().unwrap();
            }

            if scale == 0 && value >= UINT64_MIN && value <= UINT64_MAX {
                return Ok(Value::Number(Number::UInt64(u64::try_from(value).unwrap())));
            } else if scale == 0 && value >= INT64_MIN && value <= INT64_MAX {
                return Ok(Value::Number(Number::Int64(i64::try_from(value).unwrap())));
            } else if value >= DECIMAL64_MIN
                && value <= DECIMAL64_MAX
                && precision <= MAX_DECIMAL64_PRECISION
            {
                return Ok(Value::Number(Number::Decimal64(Decimal64 {
                    scale: scale as u8,
                    value: i64::try_from(value).unwrap(),
                })));
            } else if value >= DECIMAL128_MIN && value <= DECIMAL128_MAX {
                return Ok(Value::Number(Number::Decimal128(Decimal128 {
                    scale: scale as u8,
                    value,
                })));
            }
        }

        if !has_exponent && precision <= MAX_DECIMAL256_PRECISION {
            let end_idx = self.idx;

            let digit_str = if let Some(frac_idx) = fraction_offset {
                let digit_len = end_idx - num_start_idx - 1;
                let mut s = String::with_capacity(digit_len);
                s.push_str(unsafe {
                    std::str::from_utf8_unchecked(&self.buf[num_start_idx..frac_idx])
                });
                s.push_str(unsafe {
                    std::str::from_utf8_unchecked(&self.buf[frac_idx + 1..end_idx])
                });
                s
            } else {
                unsafe { std::str::from_utf8_unchecked(&self.buf[num_start_idx..end_idx]) }
                    .to_string()
            };

            if let Ok(mut value) = i256::from_str(&digit_str) {
                if negative {
                    value = value.checked_neg().unwrap();
                }
                return Ok(Value::Number(Number::Decimal256(Decimal256 {
                    scale: scale as u8,
                    value,
                })));
            }
        }

        let s = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..self.idx]) };
        match fast_float2::parse(s) {
            Ok(v) => Ok(Value::Number(Number::Float64(v))),
            Err(_) => Err(self.error(ParseErrorCode::InvalidNumberValue)),
        }
    }

    fn parse_json_string(&mut self) -> Result<Value<'a>> {
        self.must_is(b'"')?;

        let start_idx = self.idx;
        let mut escapes = 0;
        loop {
            let c = self.next()?;
            match c {
                b'\\' => {
                    self.step();
                    escapes += 1;
                    let next_c = self.next()?;
                    if *next_c == b'u' {
                        self.step();
                        let next_c = self.next()?;
                        if *next_c == b'{' {
                            self.step_by(UNICODE_LEN + 2);
                        } else {
                            self.step_by(UNICODE_LEN);
                        }
                    } else {
                        self.step();
                    }
                    continue;
                }
                b'"' => {
                    self.step();
                    break;
                }
                _ => {}
            }
            self.step();
        }

        let data = &self.buf[start_idx..self.idx - 1];
        let val = if escapes > 0 {
            let len = self.idx - 1 - start_idx - escapes;
            let mut idx = start_idx + 1;
            let s = parse_string(data, len, &mut idx)?;
            Cow::Owned(s)
        } else {
            std::str::from_utf8(data)
                .map(Cow::Borrowed)
                .map_err(|_| self.error(ParseErrorCode::InvalidStringValue))?
        };
        Ok(Value::String(val))
    }

    fn parse_json_array(&mut self) -> Result<Value<'a>> {
        self.must_is(b'[')?;

        let mut first = true;
        let mut values = Vec::new();
        loop {
            self.skip_unused();
            let c = self.next()?;
            if *c == b']' {
                self.step();
                break;
            }
            if !first {
                if *c != b',' {
                    return Err(self.error(ParseErrorCode::ExpectedArrayCommaOrEnd));
                }
                self.step();
            }
            first = false;
            self.skip_unused();
            // Check if there are consecutive commas (empty elements)
            if self.check_next_either(b',', b']') {
                // treat as null value
                values.push(Value::Null);
                continue;
            }

            let value = self.parse_json_value()?;
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn parse_json_object(&mut self) -> Result<Value<'a>> {
        self.must_is(b'{')?;

        let mut first = true;
        let mut obj = Object::new();
        loop {
            self.skip_unused();
            let c = self.next()?;
            if *c == b'}' {
                self.step();
                break;
            }
            if !first {
                if *c != b',' {
                    return Err(self.error(ParseErrorCode::ExpectedObjectCommaOrEnd));
                }
                self.step();
            }
            first = false;
            let key = self.parse_json_value()?;
            if !key.is_string() {
                return Err(self.error(ParseErrorCode::KeyMustBeAString));
            }
            self.skip_unused();
            let c = self.next()?;
            if *c != b':' {
                return Err(self.error(ParseErrorCode::ExpectedColon));
            }
            self.step();
            let value = self.parse_json_value()?;

            let k = key.as_str().unwrap();
            obj.insert(k.to_string(), value);
        }
        Ok(Value::Object(obj))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use ethnum::i256;

    fn string_strategy() -> impl Strategy<Value = String> {
        let ascii = '!'..='~';
        // CJK Unified Ideographs
        let cjk = '\u{4E00}'..='\u{9FFF}';

        let chars: Vec<char> = ascii.chain(cjk).collect();
        prop::collection::vec(prop::sample::select(chars), 1..30).prop_map(|v| v.into_iter().collect())
    }

    fn json_strategy() -> impl Strategy<Value = Value<'static>> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<u64>().prop_map(|v| Value::Number(Number::UInt64(v))),
            any::<i64>().prop_map(|v| Value::Number(Number::Int64(v))),
            any::<f64>().prop_filter("Exclude -0.0", |x| *x != -0.0).prop_map(|v| Value::Number(Number::Float64(v))),
            (0u8..19u8, any::<i64>()).prop_map(|(scale, value)| Value::Number(Number::Decimal64(Decimal64 { scale, value }))),
            (0u8..39u8, any::<i128>()).prop_map(|(scale, value)| Value::Number(Number::Decimal128(Decimal128 { scale, value }))),
            (0u8..77u8, any::<i128>(), any::<i128>()).prop_filter("Exclude big i256", 
                |(_, hi, lo)| {
                    let val = i256::from_words(*hi, *lo);
                    val >= ethnum::int!("-9999999999999999999999999999999999999999999999999999999999999999999999999999") &&
                    val <= ethnum::int!("9999999999999999999999999999999999999999999999999999999999999999999999999999")
                })
            .prop_map(|(scale, hi, lo)| Value::Number(Number::Decimal256(Decimal256 { scale, value: i256::from_words(hi, lo) }))),
            string_strategy().prop_map(|v| Value::String(Cow::Owned(v))),
        ];

        leaf.prop_recursive(
            8,
            256,
            30,
            |inner| {
                prop_oneof![
                    prop::collection::vec(inner.clone(), 0..10).prop_map(Value::Array),
                    prop::collection::btree_map(string_strategy(), inner, 0..20).prop_map(Value::Object),
                ]
            },
        )
    }

    proptest! {
        #[test]
        fn test_json_parser(json in json_strategy()) {
            let source = format!("{}", json);
            println!("source={}", source);

            let res1 = serde_json::from_slice::<serde_json::Value>(source.as_bytes());
            let res2 = parse_value(source.as_bytes());
            assert_eq!(res1.is_ok(), res2.is_ok());
            if res2.is_ok() {
                let new_json = res2.unwrap();
                let result = format!("{}", new_json);
                println!("result={}", result);
                assert_eq!(source, result);
            }
        }
    }
}


