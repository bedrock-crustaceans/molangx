//! Helpers for the rows of the `tests/measured_*.rs` topic files. A topic file needs
//! `#![cfg(feature = "compiler")]`; each helper runs the checks its enabled features allow
//! (compiling with `compiler`, evaluating with `vm`).
//!
//! A row is one builder statement. A builder ends with its terminal call, which takes the number of
//! rows it holds (`case.check(3)`, `run_NN().replay(9)`): a list row counts once per item, a setup
//! step or case-level call not at all. A wrong count or a missing terminal call fails the test. A
//! server run is built by `fn run_NN()` in its topic file and must be replayed there by a test that
//! calls `run_NN().replay(…)`.
//!
//! Server runs replay on x86-64 (on arm64 the runs up to `server::LAST_RUN_REPLAYED_UNDER_ARM64`),
//! [`EvalCase`] rows and run rows on this build's architecture.

pub mod cases;
pub mod math_call;
pub mod parse;
pub mod runs;
pub mod server;

// Not every test crate that declares `common` uses every helper.
#[allow(unused_imports)]
pub use crate::common::declared::{AllowListCase, VersionRow, VersionWindowCase};
#[allow(unused_imports)]
pub use cases::{
    Actors, CaseActor, ContextActor, EvalCase, EvalRow, HashRow, ListRow, ParseFailsRow,
    ParseFailure, ParsesRow, RangeRow,
};
#[allow(unused_imports)]
pub use parse::{ParseGroup, ParseRow};
#[allow(unused_imports)]
pub use runs::{LoopCapGroup, LoopCapRow, RunGroup, RunRow, SmokeGroup};
#[allow(unused_imports)]
pub use server::{
    BOTH_RELEASES, FIRST_RELEASE, Line, Probe, Release, ReplayReport, ServerRun, miss, text,
};
