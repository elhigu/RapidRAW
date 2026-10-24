// Rust tests for the RapidRAW backend. They are kept outside src-tauri and see only what the app
// exposes with the test-api feature. See tests/README.md.

// For the large json! literals.
#![recursion_limit = "256"]

mod cache_keys;
mod cli;
mod settings;
