//! Query resolution at lex time.

use super::Lexer;
use crate::catalog::Side;
use crate::compile::ast::{QueryRef, Span};

impl Lexer<'_, '_> {
    /// The query a name resolves to: the first version range valid for the raw version, when the
    /// options admit the query, its experiments are enabled and the catalogue's side has it.
    /// The lints it logs cover `span`.
    pub(super) fn resolve_query(&mut self, suffix: &str, span: Span) -> Option<QueryRef> {
        let opts = self.cx.opts;
        let catalog = &opts.catalog;
        let index = catalog.index_of_suffix(suffix)?;
        let decl = catalog.decl(index);
        let available = catalog.side() == Side::Client || decl.on_dedicated_server();
        let resolved = self
            .cx
            .query_version
            .and_then(|raw| decl.resolve(raw, opts.admission, opts.experiments));
        match resolved {
            Some(impl_idx) if available => {
                self.cx.lint_query_side(decl, span);
                Some(QueryRef { index, impl_idx })
            }
            Some(_) => {
                self.cx.lint_query_not_on_server(decl, span);
                None
            }
            None => {
                self.cx.lint_query_miss(decl, span);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::{QueryAdmission, QuerySetMask, Side};
    use crate::compile::{CompileOptions, Deviations, ast::Payload, lex::test_support::*};
    use crate::diag::Severity;
    use crate::stdlib::query;
    use crate::version::{ExperimentMask, MolangVersion, RawVersion};

    #[test]
    fn a_known_query_resolves_at_lex_time() {
        let r = run("q.is_baby");
        let tokens = r.tokens.unwrap();
        assert!(r.log.is_empty());
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].0, Op::QueryFunction);
        let Payload::Query(query_ref) = &tokens[0].1 else {
            panic!("{:?}", tokens[0].1)
        };
        assert_eq!(
            query_ref.index,
            crate::catalog::stdlib_index(query::IS_BABY)
        );
    }

    #[test]
    fn a_query_name_is_matched_after_lowering() {
        let r = run("Q.Is_Baby");
        let tokens = r.tokens.unwrap();
        let Payload::Query(query_ref) = &tokens[0].1 else {
            panic!()
        };
        assert_eq!(
            query_ref.index,
            crate::catalog::stdlib_index(query::IS_BABY)
        );
    }

    #[test]
    fn an_unknown_query_logs_both_messages_and_fails() {
        let r = run("1 + q.no_such");
        assert!(r.tokens.is_none());
        assert_eq!(r.log.len(), 2, "{:?}", r.log);
        // No lint for a name no query has.
        let (id, severity, span, text) = &r.log[0];
        assert_eq!(*id, "E05");
        assert_eq!(*severity, Severity::Error);
        assert_eq!(*span, (4, 13));
        assert_eq!(
            text,
            "Failed to resolve query query.no_such.  Either the query does not exist or it is not supported in this context."
        );
        assert_eq!(r.log[1].0, "E02");
        assert_eq!(r.log[1].3, "unrecognized token: q.no_such");
    }

    #[test]
    fn the_unresolved_query_message_names_the_lowered_long_spelling() {
        let r = run("Q.No_Such_Query");
        assert!(
            r.log[0]
                .3
                .starts_with("Failed to resolve query query.no_such_query."),
            "{}",
            r.log[0].3
        );
        assert_eq!(r.log[0].2, (0, 15));
    }

    #[test]
    fn an_allow_list_admits_only_its_queries() {
        let list =
            crate::catalog::QueryAllowList::new(&client().catalog, [query::IS_BABY]).unwrap();
        let opts = CompileOptions {
            admission: QueryAdmission::Only(list),
            ..client()
        };
        assert!(run_with("q.is_baby", &opts).tokens.is_some());
        let r = run_with("q.is_first_person", &opts);
        assert!(r.tokens.is_none());
        assert_eq!(r.log[0].0, "E05");
    }

    #[test]
    fn an_empty_query_set_admits_no_query() {
        let opts = CompileOptions {
            admission: QueryAdmission::Sets(QuerySetMask::empty()),
            ..client()
        };
        let r = run_with("q.is_baby", &opts);
        assert!(r.tokens.is_none());
        assert_eq!(r.log[0].0, "E05");
    }

    #[test]
    fn a_raw_version_above_thirteen_resolves_no_query() {
        let opts = CompileOptions::from_raw_version(
            crate::stdlib::queries(Side::Client).clone(),
            RawVersion(14),
        );
        let r = run_with("q.is_baby", &opts);
        assert!(r.tokens.is_none());
        assert_eq!(r.log[0].0, "-");
        assert_eq!(r.log[0].1, Severity::Info);
        assert_eq!(
            r.log[0].3,
            "query.is_baby exists, but not at MolangVersion 14"
        );
        assert_eq!(r.log[1].0, "E05");
    }

    #[test]
    fn the_miss_lint_is_off_with_the_deviation() {
        let opts = CompileOptions {
            deviations: Deviations {
                query_client_only: false,
                ..Deviations::ALL
            },
            ..CompileOptions::from_raw_version(
                crate::stdlib::queries(Side::Client).clone(),
                RawVersion(14),
            )
        };
        let r = run_with("q.is_baby", &opts);
        assert_eq!(
            r.log.iter().map(|l| l.0).collect::<Vec<_>>(),
            ["E05", "E02"]
        );
    }

    #[test]
    fn the_invalid_version_resolves_no_query() {
        let opts = CompileOptions::client(MolangVersion::Invalid);
        assert!(run_with("q.is_baby", &opts).tokens.is_none());
    }

    #[test]
    fn a_client_only_query_is_kept_on_the_server_with_a_note() {
        let opts = CompileOptions::server(MolangVersion::LATEST);
        let r = run_with("q.is_first_person", &opts);
        assert!(r.tokens.is_some());
        assert_eq!(r.log.len(), 1);
        assert_eq!(r.log[0].0, "-");
        assert_eq!(r.log[0].1, Severity::Info);
        assert_eq!(r.log[0].2, (0, 17));
        assert_eq!(
            r.log[0].3,
            "query.is_first_person is a client-only query in this crate's table; compiled for Side::Server it evaluates to its default here"
        );
    }

    #[test]
    fn a_query_the_server_does_not_have_fails_there_but_not_on_the_client() {
        let server = run_with(
            "q.is_on_screen",
            &CompileOptions::server(MolangVersion::LATEST),
        );
        assert!(server.tokens.is_none());
        assert_eq!(server.log[0].0, "-");
        assert_eq!(
            server.log[0].3,
            "query.is_on_screen is not registered on the dedicated server; it resolves only when compiled for Side::Client"
        );
        assert_eq!(server.log[1].0, "E05");
        assert!(run("q.is_on_screen").tokens.is_some());
    }

    #[test]
    fn experiments_do_not_change_a_query_without_one() {
        let opts = CompileOptions {
            experiments: ExperimentMask::empty(),
            ..client()
        };
        assert!(run_with("q.is_baby", &opts).tokens.is_some());
    }
}
