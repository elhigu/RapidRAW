# Tests

Automated tests for RapidRAW. They live here, outside the app source, so that no test code ends
up in the app.

```bash
npm test
```

This runs `cargo test --features test-api --test rust` in `src-tauri`. The desktop CI build jobs
run the same command before they build the app.

## Rust tests

`tests/rust` is a Cargo integration test of the `src-tauri` package (the `[[test]]` target in
`src-tauri/Cargo.toml`). It runs only with the `test-api` feature and does not change the app's
normal builds.

The tests use the app as the `rapidraw_lib` crate, so they see only its public items. To test a
private module, change its declaration in `src-tauri/src/lib.rs` from `mod name;` to
`test_api_mod!(name);`. This makes the module public only with the `test-api` feature.

Tests can also run the app binary, `env!("CARGO_BIN_EXE_RapidRAW")`, for example to test the
command-line export.

| File            | Tests                                                            |
| --------------- | ---------------------------------------------------------------- |
| `settings.rs`   | Old, partial and future settings files parse                     |
| `cache_keys.rs` | Which edits invalidate which of the editor's image caches        |
| `cli.rs`        | Invalid command-line export arguments fail before a window opens |

## Writing tests

- Add a file to `tests/rust` and a `mod` line for it to `tests/rust/main.rs`.
- Generate test images in the test instead of adding image files to the repository.
- Don't compare against values that change between Rust or library versions, such as
  `DefaultHasher` hashes.
- A test that needs a GPU or a display must not fail on a machine without one. It prints why it
  was skipped and returns.
