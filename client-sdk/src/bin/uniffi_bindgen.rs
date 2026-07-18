//! Thin wrapper so `cargo run --bin uniffi-bindgen -- …` drives binding
//! generation without a globally-installed `uniffi-bindgen`.
fn main() {
    uniffi::uniffi_bindgen_main()
}
