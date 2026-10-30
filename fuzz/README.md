# jsonb fuzz targets

Coverage-guided fuzzing with [cargo-fuzz] (libFuzzer + AddressSanitizer).

[cargo-fuzz]: https://github.com/rust-fuzz/cargo-fuzz

| Target | Input | Checks |
|---|---|---|
| `raw_jsonb` | Arbitrary bytes as JSONB | Every read/transform API on `RawJsonb`; a value that decodes re-encodes to stable bytes and decodes back to an equal value |
| `json_text` | Arbitrary bytes as JSON text | Both parsing modes; `parse_owned_jsonb` and `parse_value` + `to_vec` produce identical bytes; formatted output parses back to an equal value |
| `json_path` | Arbitrary bytes as a JSONPath or key-path expression | Evaluation against fixed documents; `Display` output parses back to the same path |
| `binary_ops` | Two values split at the first `0x00` byte, each parsed as JSON text when possible | Two-value APIs (`contains`, `concat`, comparisons, array/object edits); comparison is antisymmetric |

Every target must also run without panics, stack overflows, memory errors, or
allocations far larger than the input.

## Running

cargo-fuzz needs a nightly toolchain:

```sh
cargo install cargo-fuzz
cd fuzz
mkdir -p corpus/raw_jsonb
cargo +nightly fuzz run raw_jsonb corpus/raw_jsonb seeds/raw_jsonb -- \
    -dict=dicts/raw_jsonb.dict -max_len=4096 -malloc_limit_mb=512 -timeout=10 \
    -max_total_time=600
```

- The first directory receives newly discovered inputs. It is git-ignored, and
  libFuzzer exits if it does not exist, so create it before the first run.
  `seeds/<target>` holds the small committed seed corpus.
- `-dict` points libFuzzer at tokens of the input format so it reaches deeper
  code sooner. The path is resolved against the directory you run the command
  from, so run it from `fuzz/` or pass an absolute path; otherwise libFuzzer
  exits with `ParseDictionaryFile: file does not exist or is empty`. Use the
  dictionary that matches the target:

  | Target | Dictionary |
  |---|---|
  | `raw_jsonb` | `dicts/raw_jsonb.dict` |
  | `json_text`, `binary_ops` | `dicts/json.dict` |
  | `json_path` | `dicts/json_path.dict` |
- `-malloc_limit_mb` turns any single allocation above the limit into a crash,
  which catches lengths read from the input being trusted for allocation.
- `-timeout=10` reports any input that runs longer than 10 seconds as a
  `timeout-*` artifact. libFuzzer's default is 1200 seconds, so without it a
  hang looks like the fuzzer has gone quiet.
- Add `-fork=N -ignore_crashes=1` to keep fuzzing on `N` processes after a crash
  and collect every distinct failure.

libFuzzer only prints when it finds new coverage, plus `pulse` lines at
power-of-two execution counts, so long quiet periods are normal once coverage
levels off. A growing `corpus/<target>` directory means it is still working.

At the end of a run libFuzzer prints a "Recommended dictionary" of values that
helped it find new coverage. It writes non-ASCII bytes there as octal escapes,
which `-dict` files do not accept, so add them with the import script, which
converts the escapes and skips entries already present:

```sh
cargo +nightly fuzz run raw_jsonb ... 2>&1 | tee fuzz.log
python3 dicts/import_recommended.py dicts/raw_jsonb.dict < fuzz.log
```

Crashing inputs are written to `artifacts/<target>/`. To reproduce and shrink one:

```sh
cargo +nightly fuzz run raw_jsonb artifacts/raw_jsonb/crash-<hash>
cargo +nightly fuzz tmin raw_jsonb artifacts/raw_jsonb/crash-<hash>
```

When fixing a bug found this way, add the minimized input as a regression test
in `tests/it/malformed.rs`, which runs on stable with `cargo test`.
