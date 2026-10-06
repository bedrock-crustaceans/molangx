//! Differential fuzzing: generated programs on the bytecode VM and on the tree walker.
//!
//! Each case runs at its own version and at 6, 7 and 13 (the division-guard switch and the latest);
//! `generator::env::differential` states what must agree.
//!
//! ```text
//! cargo +nightly fuzz run compile_eval -- -max_total_time=600 -timeout=10
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use molangx_fuzz::generator::FuzzCase;
use molangx_fuzz::generator::env::differential;

fuzz_target!(|case: FuzzCase| {
    let mut versions = vec![case.version, 6, 7, 13];
    versions.sort_unstable();
    versions.dedup();
    for version in versions {
        let run = FuzzCase {
            version,
            ..case.clone()
        };
        if let Err(why) = differential(&run) {
            panic!("VM and tree walker disagree: {why}");
        }
    }
});
