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

const NUMBER_MAX_LEN: usize = 20;
pub const MAX_DECIMAL64_PRECISION: u8 = 18;
pub const MAX_DECIMAL128_PRECISION: u8 = 38;
pub const MAX_DECIMAL256_PRECISION: u8 = 76;

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
        let start_idx = self.idx;

/**
        let mut has_fraction = false;
        let mut has_exponent = false;
        let mut negative = false;
        let mut number_len = 0;

integer_part
fraction_part
exponent_part
*/

        let mut negative = false;
        let mut fraction_offset = None;
        let mut exponent_offset = None;

        if self.check_next(b'-') {
            negative = true;
            self.step();
        }
        if self.check_next(b'0') {
            self.step();
            if self.check_digit() {
                self.step();
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        } else {
            let len = self.step_digits()?;
            if len == 0 {
                self.step();
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }
        if self.check_next(b'.') {
            fraction_offset = Some(self.idx);
            self.step();
            let len = self.step_digits()?;
            if len == 0 {
                self.step();
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
        }
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

        // Try to parse as integer types if no fraction or exponent and number length less than max length.
        if fraction_offset.is_none() && exponent_offset.is_none() {
            let number_len = self.idx - start_idx;
            if number_len <= NUMBER_MAX_LEN {
                let s = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..self.idx]) };
                if !negative {
                    if let Ok(v) = s.parse::<u64>() {
                        return Ok(Value::Number(Number::UInt64(v)));
                    }
                } else if let Ok(v) = s.parse::<i64>() {
                    return Ok(Value::Number(Number::Int64(v)));
                }
            }
        }

        // Try to parse as decimal types first to preserve precision
        let (exp_val, exp_idx) = if let Some(exp_idx) = exponent_offset {
            let exp_str = unsafe { std::str::from_utf8_unchecked(&self.buf[exp_idx..self.idx]) };
            let Ok(exp_val) = i128::from_str(exp_str) else {
                return Err(self.error(ParseErrorCode::InvalidNumberValue));
            }
            (exp_val, exp_idx)
        } else {
            (0, self.idx)
        };

        let mut digit_buf = String::new();
        let (digit_str, scale_val) = if let Some(frac_idx) = fraction_offset {
            let int_str = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..frac_idx]) };
            let frac_str = unsafe { std::str::from_utf8_unchecked(&self.buf[frac_idx + 1..exp_idx]) };

            digit_buffer.reserve(int_str.len() + frac_str.len());
            digit_buffer.push_str(int_str);
            digit_buffer.push_str(frac_str);

            (digit_buffer.as_str(), frac_str.len())
        } else {
            let int_str = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..exp_idx]) };
            (int_str, 0)
        };

        let (scale, exp) = if scale_val >= exp_val {
            (scale_val - exp_val, 0)
        } else {
            (0, exp_val - scale)
        };
/**
        exp_val == 0
        scale 不用计算
        exp_val > 0
        scale - exp_val
        exp_val < 0
        scale + exp_val

exp_val 是负数，只需要判断 scale 是否溢出
exp_val 是正数且小于等于 scale，直接用 scale 减就可以
exp_val 是正数且大于 scale，用 exp_val - scale 之后乘以 int_num
*/
        let decimal_percision = digits.len() + exp;
        if decimal_percision <= MAX_DECIMAL128_PRECISION {
            if let Ok(digit) = i128::from_str(digit_str) {
                if exp > 0 {
                    if let Some(value) = digit.checked_mul(10.pow(exp as u32)) {
                        return Ok(Value::Number(Number::Decimal128(Decimal128 {
                            precision: 38,
                            scale: scale,
                            value,
                        })));
                    }
                }
                return Ok(Value::Number(Number::Decimal128(Decimal128 {
                    precision: 38,
                    scale: scale,
                    value,
                })));
            }
        } else if decimal_percision <= MAX_DECIMAL256_PRECISION {
            if let Ok(digit) = i256::from_str(digit_str) {
                if exp > 0 {
                    if let Some(value) = digit.checked_mul(10.pow(exp as u32)) {
                        return Ok(Value::Number(Number::Decimal256(Decimal256 {
                            precision: 38,
                            scale: scale,
                            value,
                        })));
                    }
                }
                return Ok(Value::Number(Number::Decimal256(Decimal256 {
                    precision: 38,
                    scale: scale,
                    value,
                })));
            }
        }




        let s = unsafe { std::str::from_utf8_unchecked(&self.buf[start_idx..self.idx]) };


        // Fall back to integer types if no fraction or exponent

        // Last resort: parse as float64
        match fast_float2::parse(s) {
            Ok(v) => Ok(Value::Number(Number::Float64(v))),
            Err(_) => Err(self.error(ParseErrorCode::InvalidNumberValue)),
        }
    }

    fn try_parse_as_decimal(&self, s: &str, has_fraction: bool, has_exponent: bool) -> Option<Number> {
        // For numbers with fraction or exponent, try to preserve precision with decimal types
        if has_fraction || has_exponent {
            // Parse the string to identify integer and fractional parts
            let mut parts = s.split('.');
            let int_part = parts.next().unwrap().replace(['e', 'E', '+', '-'], "");
            
            let mut scale = 0;
            let mut value_str = int_part.clone();
            
            // Handle fractional part
            if has_fraction {
                if let Some(frac_part) = parts.next() {
                    let mut frac = frac_part.to_string();
                    
                    // Handle exponent if present
                    if has_exponent {
                        if let Some(exp_idx) = frac.find(|c| c == 'e' || c == 'E') {
                            let exp_part = &frac[exp_idx + 1..];
                            frac = frac[..exp_idx].to_string();
                            
                            // Adjust scale based on exponent
                            if let Ok(exp) = exp_part.replace('+', "").parse::<i32>() {
                                scale = frac.len() as i32 - exp;
                                if scale < 0 {
                                    // Add trailing zeros for positive exponents
                                    value_str = format!("{}{}", value_str, "0".repeat(-scale as usize));
                                    scale = 0;
                                }
                            }
                        }
                    }
                    
                    if scale >= 0 {
                        scale = frac.len() as i32;
                        value_str = format!("{}{}", value_str, frac);
                    }
                }
            } else if has_exponent {
                // Handle exponent without fraction
                if let Some(exp_idx) = int_part.find(|c| c == 'e' || c == 'E') {
                    let base = &int_part[..exp_idx];
                    let exp_part = &int_part[exp_idx + 1..];
                    
                    if let Ok(exp) = exp_part.replace('+', "").parse::<i32>() {
                        if exp >= 0 {
                            // Add trailing zeros for positive exponents
                            value_str = format!("{}{}", base, "0".repeat(exp as usize));
                            scale = 0;
                        } else {
                            // For negative exponents, adjust scale
                            value_str = base.to_string();
                            scale = -exp as i32;
                        }
                    }
                }
            }
            
            // Remove any sign for parsing
            let is_negative = value_str.starts_with('-');
            let value_str = value_str.trim_start_matches('-');
            
            // Try to parse as Decimal128 first
            if let Ok(mut value) = value_str.parse::<i128>() {
                if is_negative {
                    value = -value;
                }
                
                return Some(Number::Decimal128(Decimal128 {
                    precision: 38, // Maximum precision for Decimal128
                    scale: scale as u8,
                    value,
                }));
            }
            
            // If too large for Decimal128, try Decimal256
            if let Ok(value_i256) = i256::from_str_radix(value_str, 10) {
                let value = if is_negative { -value_i256 } else { value_i256 };
                
                return Some(Number::Decimal256(Decimal256 {
                    precision: 76, // Maximum precision for Decimal256
                    scale: scale as u8,
                    value,
                }));
            }
        }
        
        None
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
