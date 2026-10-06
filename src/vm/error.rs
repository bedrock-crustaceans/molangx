//! The errors of evaluation. Neither aborts an evaluation.

use std::fmt::{self, Write as _};
use std::sync::Arc;

use thiserror::Error;

use crate::hash::HashedStr;

/// A query's report of misuse at run time: a wrong argument, a missing subject, the wrong side.
///
/// The query then returns its declared default and evaluation continues. Made with
/// [`QueryCx::error`](crate::vm::QueryCx::error), so it always names the reporting query.
///
/// ```
/// use molangx::vm::{NoHost, QueryCx, QueryResult};
///
/// fn has_any_family(cx: &mut QueryCx<'_, '_, NoHost>) -> QueryResult<NoHost> {
///     // ... argument 2 is not a string:
///     Err(cx.error(format_args!("argument {} of {} is not a string", 2, cx.name())))
/// }
/// ```
#[derive(Error, Clone, Debug, PartialEq, Eq)]
#[error("{message}")]
pub struct QueryError {
    query: Arc<str>,
    message: Box<str>,
}

impl QueryError {
    pub(crate) fn new(query: impl Into<Arc<str>>, message: impl fmt::Display) -> Self {
        // Not `to_string`, which panics on a failing `Display`; this keeps the text written so far.
        let mut text = String::new();
        let _ = write!(text, "{message}");
        Self {
            query: query.into(),
            message: text.into_boxed_str(),
        }
    }

    /// The full name of the query that reports.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The text the error displays.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A member name was added twice to a struct value.
///
/// Only the name's hash is known here, so the text prints it in hexadecimal; a host that has the
/// name reports [`RuntimeMsg::DuplicateMember`](super::RuntimeMsg::DuplicateMember) instead.
#[derive(Error, Copy, Clone, Debug, PartialEq, Eq)]
#[error("molangx: a struct already has a member named '{:#018x}'", .name.as_u64())]
pub struct DuplicateMember {
    /// The hash of the duplicated member name.
    pub name: HashedStr,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::query;

    #[test]
    fn a_query_error_displays_its_text() {
        let e = QueryError::new(
            query::HAS_ANY_FAMILY,
            format_args!(
                "argument {} of {} is not a string",
                3,
                query::HAS_ANY_FAMILY
            ),
        );
        assert_eq!(
            e.to_string(),
            "argument 3 of query.has_any_family is not a string"
        );
        assert_eq!(
            e.message(),
            "argument 3 of query.has_any_family is not a string"
        );
        assert_eq!(e.query(), query::HAS_ANY_FAMILY);
        let _: &dyn std::error::Error = &e;
    }

    #[test]
    fn a_query_error_text_has_no_placeholders() {
        for text in ["Error: %s failed", "100% {x} %d %Q", "", "日本%s語"] {
            assert_eq!(QueryError::new(query::IS_BABY, text).to_string(), text);
        }
        assert_eq!(
            QueryError::new(query::IS_BABY, String::from("owned")).message(),
            "owned"
        );
    }

    #[test]
    fn query_error_clones_and_compares() {
        let a = QueryError::new(query::IS_BABY, "one");
        assert_eq!(a.clone(), a);
        assert_ne!(a, QueryError::new(query::IS_BABY, "two"));
        assert_ne!(a, QueryError::new(query::IS_ON_FIRE, "one"));
    }

    #[test]
    fn duplicate_member_prints_the_hash_in_sixteen_hex_digits() {
        let text = |hash| {
            DuplicateMember {
                name: HashedStr::from_u64(hash),
            }
            .to_string()
        };
        assert_eq!(
            text(0xdead_beef),
            "molangx: a struct already has a member named '0x00000000deadbeef'"
        );
        assert_eq!(
            text(0),
            "molangx: a struct already has a member named '0x0000000000000000'"
        );
        assert_eq!(
            text(u64::MAX),
            "molangx: a struct already has a member named '0xffffffffffffffff'"
        );
    }

    #[test]
    fn duplicate_member_is_copy_eq_and_a_std_error() {
        let e = DuplicateMember {
            name: HashedStr::new("x"),
        };
        let copy = e;
        assert_eq!(e, copy);
        assert_ne!(
            e,
            DuplicateMember {
                name: HashedStr::new("y")
            }
        );
        let _: &dyn std::error::Error = &e;
    }
}
