//! Test tooling of `molangx`: the tree walker, a second evaluator the bytecode VM is checked
//! against, and the program generator with its differential check.
//!
//! It reads compiler internals through `molangx::internals` (feature `fuzz`), which is outside the
//! `SemVer` guarantee.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
// Numeric casts and bit-exact float comparisons are Molang's semantics.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::float_cmp)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]

pub mod generator;
pub mod tree_walker;
