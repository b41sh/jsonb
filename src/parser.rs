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
        if self.idx == self.buf.len() {
            return Err(self.error(ParseErrorCode::InvalidEOF));
        }
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
        let mut fraction_offset = None;
        let mut exponent_offset = None;

        // 处理符号
        if self.check_next(b'-') {
            negative = true;
            self.step();
        } else if self.check_next(b'+') {
            start_idx += 1;
            self.step();
        }
        
        // 处理整数部分
        self.step_digits()?;

        // 处理小数部分
        if self.check_next(b'.') {
            fraction_offset = Some(self.idx);
            self.step();
            let len = self.step_digits()?;
            if len == 0 {
                self.step();
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }
        
        // 处理指数部分
        if self.check_next_either(b'E', b'e') {
            exponent_offset = Some(self.idx);
            self.step();
            if self.check_next_either(b'+', b'-') {
                self.step();
            }
            let len = self.step_digits()?;
            if len == 0 {
                self.step();
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }

        // 快速路径：如果没有小数点和指数，尝试解析为整数类型
        if fraction_offset.is_none() && exponent_offset.is_none() {
            // 尝试直接从字节解析为整数，避免字符串转换
            if let Some(value) = self.try_parse_integer(start_idx, negative) {
                return Ok(value);
            }
        }

        // 尝试解析为十进制类型
        if let Some(value) = self.try_parse_decimal(start_idx, fraction_offset, exponent_offset, negative) {
            return Ok(value);
        }

        // 最后尝试解析为浮点数
        let s = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..self.idx]) };
        match fast_float2::parse(s) {
            Ok(v) => Ok(Value::Number(Number::Float64(v))),
            Err(_) => Err(self.error(ParseErrorCode::InvalidNumberValue)),
        }
    }

    // 新增辅助方法：直接从字节解析为整数
    fn try_parse_integer(&self, start_idx: usize, negative: bool) -> Option<Value<'a>> {
        let integer_len = self.idx - start_idx;
        if integer_len > MAX_INTGER_PRECISION {
            return None;
        }

        if negative {
            let value = self.parse_digits_to_number::<i64>(start_idx, self.idx, negative, false)?;
            Some(Value::Number(Number::Int64(value)))
        } else {
            let value = self.parse_digits_to_number::<u64>(start_idx, self.idx, negative, false)?;
            Some(Value::Number(Number::UInt64(value)))
        }
    }

    fn try_parse_decimal(&mut self, start_idx: usize, fraction_offset: Option<usize>, exponent_offset: Option<usize>, negative: bool) -> Option<Value<'a>> {
        // 解析指数部分
        let (exp_val, exp_idx) = if let Some(exp_idx) = exponent_offset {
            let exp_str = unsafe { std::str::from_utf8_unchecked(&self.buf[exp_idx+1..self.idx]) };
            if let Ok(exp_val) = exp_str.parse::<i32>() {
                (exp_val, exp_idx)
            } else {
                return None;
            }
        } else {
            (0, self.idx)
        };

        // 计算小数部分长度
        let scale_val = if let Some(frac_idx) = fraction_offset {
            (exp_idx - frac_idx - 1) as i32
        } else {
            0
        };

        // 计算最终的精度和缩放
        let (scale, exp) = if scale_val >= exp_val {
            ((scale_val - exp_val) as usize, 0)
        } else {
            (0, (exp_val - scale_val) as u32)
        };

        // 计算数字的总长度（不包括小数点和指数部分）
        let digit_len = if let Some(frac_idx) = fraction_offset {
            (frac_idx - start_idx) + (exp_idx - frac_idx - 1)
        } else {
            exp_idx - start_idx
        };

        let precision = digit_len + exp as usize;
        
        // 首先尝试解析为 i128 (Decimal128)
        if precision <= MAX_DECIMAL128_PRECISION && scale <= MAX_DECIMAL128_PRECISION {
            // 直接从字节解析为数字，避免字符串转换
            if let Some(value) = self.parse_digits_to_i128(start_idx, exp_idx, exp, negative) {
                return Some(Value::Number(Number::Decimal128(Decimal128 {
                    precision: 38,
                    scale: scale as u8,
                    value,
                })));
            }
        }

        // 如果需要更高精度，尝试解析为 i256 (Decimal256)
        if precision <= MAX_DECIMAL256_PRECISION && scale <= MAX_DECIMAL256_PRECISION {
            // 对于 i256，我们仍然需要通过字符串解析，因为没有直接的字节到 i256 的转换
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
        }

        None
    }

    // 新增辅助方法：直接从字节解析为 i128，避免字符串转换
    fn parse_digits_to_i128(&self, start_idx: usize, end_idx: usize, exp: u32, negative: bool) -> Option<i128> {
        let value = self.parse_digits_to_number::<i128>(start_idx, end_idx, negative, true)?;
        // 应用指数
        if exp > 0 {
            value.checked_mul(10_i128.pow(exp))
        } else {
            Some(value)
        }
    }


    // 通用辅助方法：从字节解析为数字类型
    fn parse_digits_to_number<T>(&self, start_idx: usize, end_idx: usize, negative: bool, skip_dot: bool) -> Option<T> 
    where 
        T: num_traits::PrimInt + num_traits::CheckedMul + num_traits::CheckedAdd + num_traits::CheckedNeg,
    {
        let mut value: T = T::zero();
        let mut i = start_idx;
        
        // 跳过符号
        if negative {
            i += 1;
        }
        
        // 解析数字
        while i < end_idx {
            // 跳过小数点
            if skip_dot && self.buf[i] == b'.' {
                i += 1;
                continue;
            }
            
            let digit_byte = self.buf[i] - b'0';
            if digit_byte > 9 { // 非数字字符
                return None;
            }
            
            let digit = T::from(digit_byte).unwrap();
            
            // 检查乘法溢出
            if let Some(new_value) = value.checked_mul(&T::from(10).unwrap()) {
                if let Some(new_value) = new_value.checked_add(&digit) {
                    value = new_value;
                } else {
                    return None; // 加法溢出
                }
            } else {
                return None; // 乘法溢出
            }
            
            i += 1;
        }
        
        // 应用符号
        if negative {
            value.checked_neg()
        } else {
            Some(value)
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

