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

// JSONB header constants
pub(crate) const ARRAY_PREFIX: u8 = 0x80;
pub(crate) const OBJECT_PREFIX: u8 = 0x40;
pub(crate) const SCALAR_PREFIX: u8 = 0x20;

pub(crate) const ARRAY_CONTAINER_TAG: u32 = 0x80000000;
pub(crate) const OBJECT_CONTAINER_TAG: u32 = 0x40000000;
pub(crate) const SCALAR_CONTAINER_TAG: u32 = 0x20000000;

pub(crate) const CONTAINER_HEADER_TYPE_MASK: u32 = 0xE0000000;
pub(crate) const CONTAINER_HEADER_LEN_MASK: u32 = 0x1FFFFFFF;

// JSONB JEntry constants
pub(crate) const NULL_TAG: u32 = 0x00000000;
pub(crate) const STRING_TAG: u32 = 0x10000000;
pub(crate) const NUMBER_TAG: u32 = 0x20000000;
pub(crate) const FALSE_TAG: u32 = 0x30000000;
pub(crate) const TRUE_TAG: u32 = 0x40000000;
pub(crate) const CONTAINER_TAG: u32 = 0x50000000;

// JSONB number constants
pub(crate) const NUMBER_ZERO: u8 = 0x00;
pub(crate) const NUMBER_NAN: u8 = 0x10;
pub(crate) const NUMBER_INF: u8 = 0x20;
pub(crate) const NUMBER_NEG_INF: u8 = 0x30;
pub(crate) const NUMBER_INT: u8 = 0x40;
pub(crate) const NUMBER_UINT: u8 = 0x50;
pub(crate) const NUMBER_FLOAT: u8 = 0x60;

// @todo support offset mode
#[allow(dead_code)]
pub(crate) const JENTRY_IS_OFF_FLAG: u32 = 0x80000000;
pub(crate) const JENTRY_TYPE_MASK: u32 = 0x70000000;
pub(crate) const JENTRY_OFF_LEN_MASK: u32 = 0x0FFFFFFF;

// JSON text constants
pub(crate) const UNICODE_LEN: usize = 4;

// JSON text escape characters constants
pub(crate) const BS: char = '\x5C'; // \\ Backslash
pub(crate) const QU: char = '\x22'; // \" Double quotation mark
pub(crate) const SD: char = '\x2F'; // \/ Slash or divide
pub(crate) const BB: char = '\x08'; // \b Backspace
pub(crate) const FF: char = '\x0C'; // \f Formfeed Page Break
pub(crate) const NN: char = '\x0A'; // \n Newline
pub(crate) const RR: char = '\x0D'; // \r Carriage Return
pub(crate) const TT: char = '\x09'; // \t Horizontal Tab

// JSONB value compare level
pub(crate) const NULL_LEVEL: u8 = 7;
pub(crate) const ARRAY_LEVEL: u8 = 6;
pub(crate) const OBJECT_LEVEL: u8 = 5;
pub(crate) const STRING_LEVEL: u8 = 4;
pub(crate) const NUMBER_LEVEL: u8 = 3;
pub(crate) const TRUE_LEVEL: u8 = 2;
pub(crate) const FALSE_LEVEL: u8 = 1;
pub(crate) const INVALID_LEVEL: u8 = 0;
