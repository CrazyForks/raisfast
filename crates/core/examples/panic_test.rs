//! Panic hook smoke test — run with: cargo run --example panic_test

#![deny(unsafe_code)]

use raisfast::panic_hook;

fn main() {
    let log_dir = std::env::temp_dir().join("raisfast_panic_test");
    let _ = std::fs::remove_dir_all(&log_dir);
    std::fs::create_dir_all(&log_dir).unwrap();

    println!(">> Installing panic hook (log_dir={})", log_dir.display());
    panic_hook::install(log_dir.to_str().unwrap());

    println!(">> About to panic...");
    panic!("boom! intentional panic for testing");
}
