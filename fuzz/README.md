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
cargo +nightly fuzz run raw_jsonb corpus/raw_jsonb seeds/raw_jsonb -- \
    -max_len=4096 -malloc_limit_mb=512 -max_total_time=600
```

- The first directory receives newly discovered inputs; it is git-ignored.
  `seeds/<target>` holds the small committed seed corpus.
- `-malloc_limit_mb` turns any single allocation above the limit into a crash,
  which catches lengths read from the input being trusted for allocation.
- Add `-fork=N -ignore_crashes=1` to keep fuzzing on `N` processes after a crash
  and collect every distinct failure.

Crashing inputs are written to `artifacts/<target>/`. To reproduce and shrink one:

```sh
cargo +nightly fuzz run raw_jsonb artifacts/raw_jsonb/crash-<hash>
cargo +nightly fuzz tmin raw_jsonb artifacts/raw_jsonb/crash-<hash>
```

When fixing a bug found this way, add the minimized input as a regression test
in `tests/it/malformed.rs`, which runs on stable with `cargo test`.
