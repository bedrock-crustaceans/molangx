//! Arbitrary text through the compiler front end: the first two bytes choose the version and the
//! options, the rest is the source; `generator::front_end::check` states what must hold.
//!
//! Each input compiles on a 256 KiB stack, so recursion that grows with the input's nesting
//! overflows long before an 8 MiB main thread would. A per-input timeout catches work superlinear
//! in the input's length. Run with the committed seeds as a read-only second corpus:
//!
//! ```text
//! cargo +nightly fuzz run lex_parse fuzz/corpus/lex_parse fuzz/seeds/lex_parse -- -max_total_time=900 -timeout=2 -max_len=65538
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use molangx_fuzz::generator::front_end::check;

const STACK: usize = 256 * 1024;

fuzz_target!(|data: &[u8]| {
    let [version, flags, source @ ..] = data else {
        return;
    };
    let Ok(source) = std::str::from_utf8(source) else {
        return;
    };
    let (version, flags, source) = (*version, *flags, source.to_owned());
    let worker = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || check(version, flags, &source))
        .expect("cannot spawn the compile thread");
    if let Err(panic) = worker.join() {
        std::panic::resume_unwind(panic);
    }
});
