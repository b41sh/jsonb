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

mod array;
mod core;
mod object;
mod operator;
mod path;
mod scalar;

--
|
---

https://github.com/databendlabs/jsonb/pull/77
看下这个 PR，把编码相关的操作提取出来，简化了函数的实现
代码结构变成了这样
├── core
│   ├── databend
│   │   ├── builder.rs
│   │   ├── de.rs
│   │   ├── iterator.rs
│   │   ├── mod.rs
│   │   └── ser.rs
│   ├── mod.rs
│   └── sqlite
│       └── mod.rs
├── functions
│   ├── array.rs
│   ├── core.rs
│   ├── mod.rs
│   ├── object.rs
│   ├── operator.rs
│   ├── path.rs
│   └── scalar.rs
├── jsonpath
│   ├── mod.rs
│   ├── parser.rs
│   ├── path.rs
│   └── selector.rs
├── owned.rs
├── raw.rs
    ...