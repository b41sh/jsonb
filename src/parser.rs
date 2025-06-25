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
 
use ethnum::i256;
use crate::Decimal128;
use crate::Decimal256;

pub const MAX_EXPONENT_PRECISION: usize = 9;
pub const MAX_INTGER_PRECISION: usize = 18;
pub const MAX_DECIMAL64_PRECISION: usize = 18;
pub const MAX_DECIMAL128_PRECISION: usize = 38;
pub const MAX_DECIMAL256_PRECISION: usize = 76;

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
            b'0'..=b'9' | b'-' => self.parse_json_number(),
            b'"' => self.parse_json_string(),
            b'[' => self.parse_json_array(),
            b'{' => self.parse_json_object(),
            _ => {
                self.step();
                Err(self.error(ParseErrorCode::ExpectedSomeValue))
            }
        }
    }

    fn next(&mut self) -> Result<&u8> {
        match self.buf.get(self.idx) {
            Some(c) => Ok(c),
            None => Err(self.error(ParseErrorCode::InvalidEOF)),
        }
    }

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

    fn check_next(&mut self, c: u8) -> bool {
        if self.idx < self.buf.len() {
            let v = self.buf.get(self.idx).unwrap();
            if v == &c {
                return true;
            }
        }
        false
    }

    fn check_next_either(&mut self, c1: u8, c2: u8) -> bool {
        if self.idx < self.buf.len() {
            let v = self.buf.get(self.idx).unwrap();
            if v == &c1 || v == &c2 {
                return true;
            }
        }
        false
    }

    fn check_digit(&mut self) -> bool {
        if self.idx < self.buf.len() {
            let v = self.buf.get(self.idx).unwrap();
            if v.is_ascii_digit() {
                return true;
            }
        }
        false
    }

    fn step_digits(&mut self) -> Result<usize> {
        //if self.idx == self.buf.len() {
        //    return Err(self.error(ParseErrorCode::InvalidEOF));
        //}
        let mut len = 0;
        while self.idx < self.buf.len() {
            let c = self.buf.get(self.idx).unwrap();
            if !c.is_ascii_digit() {
                break;
            }
            len += 1;
            self.step();
        }
        Ok(len)
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
            let c = self.buf.get(self.idx).unwrap();
            if c.is_ascii_whitespace() {
                self.step();
                continue;
            }
            // Allow parse escaped white space
            if *c == b'\\' {
                if self.idx + 1 < self.buf.len()
                    && matches!(self.buf[self.idx + 1], b'n' | b'r' | b't')
                {
                    self.step_by(2);
                    continue;
                }
                if self.idx + 3 < self.buf.len()
                    && self.buf[self.idx + 1] == b'x'
                    && self.buf[self.idx + 2] == b'0'
                    && self.buf[self.idx + 3] == b'C'
                {
                    self.step_by(4);
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

    fn parse_json_number(&mut self) -> Result<Value<'a>> {
        let mut start_idx = self.idx;

        let mut negative = false;

        // 处理符号
        let c = self.next()?;
        if *c == b'-' {
            negative = true;
            self.step();
        } else if *c == b'+' {
            self.step();
        }

        // ignore leading zeros
        loop {
            if self.check_next(b'0') {
                self.step();
            } else {
                break;
            }
        }

        let mut value = 0_i128;
        let mut scale = 0_i32;
        let mut fraction_offset = None;
        //let mut exponent_offset = None;

        let mut precision = 0;
        // 首先尝试解析 i256 的数字，避免重复遍历
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

        // 有三种情况
        // 1. 整数还没处理完，已经溢出了
        // 2. 整数处理完了，浮点数还没开始
        // 3. 整数和浮点数部分都已经处理完了，再看看有没有 e

        // precision 小于最大值，说明整数部分和小数部分都已经处理完成
        if precision < MAX_DECIMAL128_PRECISION {
            let exp = self.parse_exponent_value()?;
            if let Some(exp) = exp {
                let (new_scale, exp) = if scale >= exp {
                    ((scale - exp), 0)
                } else {
                    (0, (exp - scale) as u32)
                };
                precision += exp as usize;
                scale = new_scale;

                // 首先尝试解析为 i128 (Decimal128)
                if precision <= MAX_DECIMAL128_PRECISION {
                    if exp > 0 {
                        value = unsafe { value.unchecked_mul(10_i128.pow(exp)) };
                    }
                    if negative {
                        value = value.checked_neg().unwrap();
                    }
                    if scale == 0 && value >= 0 && value <= i128::from(u64::MAX) {
                        return Ok(Value::Number(Number::UInt64(u64::try_from(value).unwrap())));
                    } else if scale == 0 && value >= i128::from(i64::MIN) && value <= i128::from(i64::MAX) {
                        return Ok(Value::Number(Number::Int64(i64::try_from(value).unwrap())));
                    } else {
                        return Ok(Value::Number(Number::Decimal128(Decimal128 {
                            precision: 38,
                            scale: scale as u8,
                            value,
                        })));
                    }
                }
            }
        } else {
            // precision 已经超过 i128 的最大值，回退到原来的方式处理
            if fraction_offset.is_none() {
                // 处理整数部分
                let len = self.step_digits()?;
                precision += len;
                if self.check_next(b'.') {
                    fraction_offset = Some(self.idx);
                    self.step();
                }
            }

            if fraction_offset.is_some() {
                let len = self.step_digits()?;
                precision += len;
                scale += len as i32;
                if scale == 0 {
                    return Err(self.error(ParseErrorCode::InvalidNumberValue));
                }
            }
            
            let exp = self.parse_exponent_value()?;
            if let Some(exp) = exp {
                let (new_scale, exp) = if scale >= exp {
                    ((scale - exp), 0)
                } else {
                    (0, (exp - scale) as u32)
                };
                precision += exp as usize;
                scale = new_scale;
            }
        }

        // 如果需要更高精度，尝试解析为 i256 (Decimal256)
        if precision <= MAX_DECIMAL256_PRECISION {
            // 对于 i256，我们仍然需要通过字符串解析，因为没有直接的字节到 i256 的转换
            /**
            let digit_str = if let Some(frac_idx) = fraction_offset {
                let mut s = String::with_capacity(digit_len);
                s.push_str(unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..frac_idx]) });
                s.push_str(unsafe { std::str::from_utf8_unchecked(&self.buf[frac_idx+1..exp_idx]) });
                s
            } else {
                unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..exp_idx]) }.to_string()
            };

            if let Ok(value) = i256::from_str(&digit_str) {
                if let Some(value) = value.checked_mul(i256::from(10).pow(exp)) {
                    return Some(Value::Number(Number::Decimal256(Decimal256 {
                        precision: 76,
                        scale: scale as u8,
                        value,
                    })));
                }
            }
            */
            todo!()
        }

        // 最后尝试解析为浮点数
        let s = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..self.idx]) };
        match fast_float2::parse(s) {
            Ok(v) => Ok(Value::Number(Number::Float64(v))),
            Err(_) => Err(self.error(ParseErrorCode::InvalidNumberValue)),
        }
    }



    fn parse_exponent_value(&mut self) -> Result<Option<i32>> {
        if self.check_next_either(b'E', b'e') {
            self.step();
            let mut negative = false;
            let c = self.next()?;
            if *c == b'-' {
                negative = true;
                self.step();
            } else if *c == b'+' {
                self.step();
            }
            let mut i = 0;
            let mut exp_value = 0_i32;
            while i < MAX_EXPONENT_PRECISION {
                if self.check_digit() {
                    let digit = (self.buf[self.idx] - b'0') as i32;

                    exp_value = unsafe { exp_value.unchecked_mul(10_i32) };
                    exp_value = unsafe { exp_value.unchecked_add(digit) };
                    self.step();
                } else {
                    break;
                }
                i += 1;
            }
            if i == 0 {
                Err(self.error(ParseErrorCode::InvalidNumberValue))
            } else if i < MAX_EXPONENT_PRECISION {
                if negative {
                    Ok(exp_value.checked_neg())
                } else {
                    Ok(Some(exp_value))
                }
            } else {
                loop {
                    if self.check_digit() {
                        self.step();
                    } else {
                        break;
                    }
                }
                Ok(None)
            }
        } else {
            Ok(Some(0))
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
            // 检查是否有连续的逗号（空元素）
            if self.check_next_either(b',', b']') {
                // 发现空元素，添加 null 值
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

