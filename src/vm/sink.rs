//! Run-time diagnostics: the evaluator's messages and the sinks they go to.

use std::collections::{HashSet, VecDeque};
use std::fmt::{self, Write as _};

use nohash_hasher::BuildNoHashHasher;

use super::error::QueryError;
use crate::hash::{FNV1_OFFSET_BASIS, fnv1_step};

/// The level of a run-time message, from less to more severe.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// A message of molangx itself (`molangx: …`).
    Warn,
    /// One of Molang's run-time errors.
    Error,
}

/// A message of the evaluator itself; `Display` gives its text, formatted only on demand.
///
/// [`UnknownVariable`](Self::UnknownVariable) and [`MissingMember`](Self::MissingMember) are
/// Molang's run-time errors; the other texts start with `molangx: `.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuntimeMsg<'a> {
    /// A name or struct member was read while unset with no enclosing `??`; the expression ends
    /// with 0. Inside `->` the text adds a hint about public variables.
    UnknownVariable {
        /// The full canonical name, e.g. `variable.x`.
        name: &'a str,
        /// Whether the evaluation was in public-access mode (inside `->`).
        public_access: bool,
    },
    /// A struct member was read that the struct does not have; the missing-variable path follows.
    MissingMember {
        /// The member's name.
        name: &'a str,
    },
    /// For a host that reports a value of the wrong kind; never sent by the evaluator.
    IncompatibleType {
        /// The name the text ends with, if any.
        name: Option<&'a str>,
    },
    /// The `->` bookkeeping closed a public-access scope while none was open.
    PublicAccessUnderflow,
    /// A member was added twice to a struct; sent by hosts that build structs, never by the
    /// evaluator.
    DuplicateMember {
        /// The duplicated member name.
        name: &'a str,
    },
    /// A `loop` / `for_each` used up its iteration budget and was left; sent only for the first
    /// such loop of an evaluation.
    LoopLimit {
        /// The per-loop budget that was exhausted.
        limit: u32,
    },
    /// The evaluation used up its step budget and ended with 0.
    StepLimit {
        /// The per-evaluation budget that was exhausted.
        limit: u64,
    },
    /// A member store would have nested a struct deeper than its budget; nothing was written and
    /// the evaluation ended with 0.
    StructDepthLimit {
        /// The struct nesting budget.
        limit: u32,
    },
    /// Operands left behind by `break` / `continue` filled the operand stack past its cap; the
    /// evaluation ended with 0.
    OperandStackLimit {
        /// The cap ([`EvalLimits::OPERAND_STACK_CAP`](super::EvalLimits::OPERAND_STACK_CAP)).
        limit: u32,
    },
    /// A member store would have added a member to a full struct; nothing was written and the
    /// evaluation ended with 0.
    StructMemberLimit {
        /// The struct width budget
        /// ([`EvalLimits::struct_members`](super::EvalLimits::struct_members)).
        limit: u32,
    },
    /// A query argument would run deeper than
    /// [`EvalLimits::query_depth`](super::EvalLimits::query_depth); the evaluation ended with 0.
    QueryDepthLimit {
        /// The query-argument nesting budget.
        limit: u32,
    },
}

impl RuntimeMsg<'_> {
    /// [`LogLevel::Error`] for Molang's run-time errors, [`LogLevel::Warn`] for a `molangx: `
    /// message.
    pub const fn level(&self) -> LogLevel {
        match self {
            Self::UnknownVariable { .. } | Self::MissingMember { .. } => LogLevel::Error,
            _ => LogLevel::Warn,
        }
    }
}

impl fmt::Display for RuntimeMsg<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnknownVariable {
                name,
                public_access: false,
            } => write!(f, "Error: unhandled request for unknown variable '{name}'"),
            Self::UnknownVariable {
                name,
                public_access: true,
            } => write!(
                f,
                "Error: unhandled request for unknown variable '{name}' - are you trying to access a variable from a different mob that hasn't made its variable public in its resource definition?"
            ),
            Self::MissingMember { name } => {
                write!(f, "Error: unable to find member variable {name}")
            }
            Self::IncompatibleType { name: None } => {
                f.write_str("molangx: a host value has an incompatible type")
            }
            Self::IncompatibleType { name: Some(name) } => {
                write!(f, "molangx: a host value has an incompatible type: {name}")
            }
            Self::PublicAccessUnderflow => {
                f.write_str("molangx: a public-access scope was closed while none was open")
            }
            Self::DuplicateMember { name } => {
                write!(f, "molangx: a struct already has a member named '{name}'")
            }
            Self::LoopLimit { limit } => write!(
                f,
                "molangx: loop stopped after its budget of {limit} iterations"
            ),
            Self::StepLimit { limit } => write!(
                f,
                "molangx: evaluation stopped after its budget of {limit} steps"
            ),
            Self::StructDepthLimit { limit } => write!(
                f,
                "molangx: evaluation stopped: a struct would nest deeper than its budget of {limit} levels"
            ),
            Self::OperandStackLimit { limit } => write!(
                f,
                "molangx: evaluation stopped: operands left behind by break / continue passed the cap of {limit}"
            ),
            Self::StructMemberLimit { limit } => write!(
                f,
                "molangx: evaluation stopped: a struct would hold more than its budget of {limit} members"
            ),
            Self::QueryDepthLimit { limit } => write!(
                f,
                "molangx: evaluation stopped: query arguments would nest deeper than their budget of {limit} levels"
            ),
        }
    }
}

/// Where run-time diagnostics go. Reporting never aborts an evaluation.
pub trait RuntimeSink {
    /// A message of the evaluator itself.
    fn runtime(&mut self, msg: RuntimeMsg<'_>);

    /// A query reported a misuse; the call's value is the query's no-subject default.
    fn query_error(&mut self, error: QueryError);
}

impl<S: RuntimeSink + ?Sized> RuntimeSink for &mut S {
    fn runtime(&mut self, msg: RuntimeMsg<'_>) {
        (**self).runtime(msg);
    }

    fn query_error(&mut self, error: QueryError) {
        (**self).query_error(error);
    }
}

/// A sink that drops everything.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NullSink;

impl RuntimeSink for NullSink {
    fn runtime(&mut self, _msg: RuntimeMsg<'_>) {}

    fn query_error(&mut self, _error: QueryError) {}
}

/// A sink that keeps the last [`BoundedSink::DEFAULT_CAPACITY`] messages as text, each cut to
/// [`BoundedSink::MAX_MESSAGE_BYTES`], and counts the ones it let go; the default sink.
///
/// Its memory stays bounded although a message can embed a 64 KiB name and content can log on
/// every evaluation. It keeps repeats; wrap it in [`LogOnce`] to drop them.
///
/// ```
/// use molangx::vm::{BoundedSink, RuntimeMsg, RuntimeSink};
///
/// let mut sink = BoundedSink::with_capacity(2);
/// for name in ["variable.a", "variable.b", "variable.c"] {
///     sink.runtime(RuntimeMsg::UnknownVariable { name, public_access: false });
/// }
/// assert_eq!(sink.dropped(), 1);
/// assert_eq!(sink.take(), [
///     "Error: unhandled request for unknown variable 'variable.b'",
///     "Error: unhandled request for unknown variable 'variable.c'",
/// ]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedSink {
    messages: VecDeque<String>,
    capacity: usize,
    dropped: u64,
}

impl BoundedSink {
    /// How many messages [`BoundedSink::new`] keeps.
    pub const DEFAULT_CAPACITY: usize = 64;

    /// The most bytes of one message the sink keeps; a longer text is cut at a character boundary
    /// and ends with `…`.
    pub const MAX_MESSAGE_BYTES: usize = 1024;

    /// An empty sink that keeps the last [`BoundedSink::DEFAULT_CAPACITY`] messages.
    pub const fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// An empty sink that keeps the last `capacity` messages (0: only counts them).
    pub const fn with_capacity(capacity: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            capacity,
            dropped: 0,
        }
    }

    /// The kept messages, oldest first.
    pub fn messages(&self) -> &VecDeque<String> {
        &self.messages
    }

    /// Whether no message is kept.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// How many messages were let go since the sink was made or last taken from.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Takes the kept messages, oldest first, leaving the sink empty and its dropped count 0.
    pub fn take(&mut self) -> Vec<String> {
        self.dropped = 0;
        std::mem::take(&mut self.messages).into()
    }

    fn keep(&mut self, text: &dyn fmt::Display) {
        if self.capacity == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if self.messages.len() >= self.capacity {
            self.messages.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        let mut capped = Capped::default();
        let _ = write!(capped, "{text}");
        self.messages.push_back(capped.finish());
    }
}

impl Default for BoundedSink {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeSink for BoundedSink {
    fn runtime(&mut self, msg: RuntimeMsg<'_>) {
        self.keep(&msg);
    }

    fn query_error(&mut self, error: QueryError) {
        self.keep(&error);
    }
}

/// A text buffer that keeps the first [`BoundedSink::MAX_MESSAGE_BYTES`] bytes written to it.
#[derive(Default)]
struct Capped {
    text: String,
    cut: bool,
}

impl Capped {
    /// The kept text, ending with `…` when something was cut.
    fn finish(mut self) -> String {
        if self.cut {
            self.text.push('…');
        }
        self.text
    }
}

impl fmt::Write for Capped {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.cut {
            return Ok(());
        }
        let room = BoundedSink::MAX_MESSAGE_BYTES - self.text.len();
        if s.len() <= room {
            self.text.push_str(s);
        } else {
            self.text.push_str(&s[..s.floor_char_boundary(room)]);
            self.cut = true;
        }
        Ok(())
    }
}

/// A sink that keeps every message as text, whole and in order.
///
/// Unbounded: content that logs on every evaluation grows it without limit, so use
/// [`BoundedSink`] in a long-running host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectSink {
    /// The formatted messages, oldest first.
    pub messages: Vec<String>,
}

impl CollectSink {
    /// An empty sink.
    pub const fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// Whether nothing was reported.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Takes the collected messages, leaving the sink empty.
    pub fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }
}

impl RuntimeSink for CollectSink {
    fn runtime(&mut self, msg: RuntimeMsg<'_>) {
        self.messages.push(msg.to_string());
    }

    fn query_error(&mut self, error: QueryError) {
        self.messages.push(error.to_string());
    }
}

/// A sink that passes on only the first message with each formatted text in a session.
///
/// Texts are remembered by hash, without allocating. When a new text arrives while
/// [`LogOnce::SEEN_CAP`] hashes are held, the set is emptied first, as by [`LogOnce::reset`].
#[derive(Clone, Debug, Default)]
pub struct LogOnce<S> {
    inner: S,
    seen: HashSet<u64, BuildNoHashHasher<u64>>,
}

impl<S: RuntimeSink> LogOnce<S> {
    /// The most message hashes one session remembers.
    pub const SEEN_CAP: usize = 4096;

    /// Wraps `inner`; nothing has been seen yet.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            seen: HashSet::default(),
        }
    }

    /// The inner sink.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// The inner sink, mutably.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Unwraps the inner sink.
    pub fn into_inner(self) -> S {
        self.inner
    }

    /// Starts a new session: every message may be passed on once again.
    pub fn reset(&mut self) {
        self.seen.clear();
    }

    /// Number of distinct messages seen in this session.
    pub fn distinct(&self) -> usize {
        self.seen.len()
    }

    /// Records the text; `true` when it is new.
    fn first_time(&mut self, text: &dyn fmt::Display) -> bool {
        let mut hasher = FnvWriter::new();
        let _ = write!(hasher, "{text}");
        let hash = hasher.finish();
        if self.seen.contains(&hash) {
            return false;
        }
        if self.seen.len() >= Self::SEEN_CAP {
            self.seen.clear();
        }
        self.seen.insert(hash)
    }
}

impl<S: RuntimeSink> RuntimeSink for LogOnce<S> {
    fn runtime(&mut self, msg: RuntimeMsg<'_>) {
        if self.first_time(&msg) {
            self.inner.runtime(msg);
        }
    }

    fn query_error(&mut self, error: QueryError) {
        if self.first_time(&error) {
            self.inner.query_error(error);
        }
    }
}

/// Streaming [`HashedStr`](crate::hash::HashedStr) hashing: 0 for no bytes.
struct FnvWriter {
    hash: u64,
    any: bool,
}

impl FnvWriter {
    const fn new() -> Self {
        Self {
            hash: FNV1_OFFSET_BASIS,
            any: false,
        }
    }

    const fn finish(&self) -> u64 {
        if self.any { self.hash } else { 0 }
    }
}

impl fmt::Write for FnvWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &byte in s.as_bytes() {
            self.hash = fnv1_step(self.hash, byte);
            self.any = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;
    use crate::hash::HashedStr;
    use crate::stdlib::query;

    fn unknown(name: &str) -> RuntimeMsg<'_> {
        RuntimeMsg::UnknownVariable {
            name,
            public_access: false,
        }
    }

    fn error(message: &str) -> QueryError {
        QueryError::new(query::IS_BABY, message)
    }

    #[test]
    fn log_levels_are_ordered_by_severity() {
        assert!(LogLevel::Error > LogLevel::Warn);
    }

    #[test]
    fn the_two_recorded_messages_keep_their_texts() {
        let cases = [
            (
                RuntimeMsg::UnknownVariable {
                    name: "variable.missing_never_set_r16",
                    public_access: false,
                },
                "Error: unhandled request for unknown variable 'variable.missing_never_set_r16'",
            ),
            (
                RuntimeMsg::UnknownVariable {
                    name: "temp.x",
                    public_access: true,
                },
                "Error: unhandled request for unknown variable 'temp.x' - are you trying to access a variable from a different mob that hasn't made its variable public in its resource definition?",
            ),
            (
                RuntimeMsg::MissingMember { name: "b" },
                "Error: unable to find member variable b",
            ),
        ];
        for (msg, text) in cases {
            assert_eq!(msg.to_string(), text);
            assert_eq!(msg.level(), LogLevel::Error);
        }
    }

    #[test]
    fn a_missing_member_message_embeds_the_name_as_given() {
        assert_eq!(
            RuntimeMsg::MissingMember { name: ".b" }.to_string(),
            "Error: unable to find member variable .b"
        );
        assert_eq!(
            RuntimeMsg::MissingMember { name: "" }.to_string(),
            "Error: unable to find member variable "
        );
    }

    #[test]
    fn our_message_texts_are_exact() {
        let cases = [
            (
                RuntimeMsg::IncompatibleType { name: None },
                "molangx: a host value has an incompatible type",
            ),
            (
                RuntimeMsg::IncompatibleType {
                    name: Some("texture.default"),
                },
                "molangx: a host value has an incompatible type: texture.default",
            ),
            (
                RuntimeMsg::PublicAccessUnderflow,
                "molangx: a public-access scope was closed while none was open",
            ),
            (
                RuntimeMsg::DuplicateMember { name: "x" },
                "molangx: a struct already has a member named 'x'",
            ),
            (
                RuntimeMsg::LoopLimit { limit: 1024 },
                "molangx: loop stopped after its budget of 1024 iterations",
            ),
            (
                RuntimeMsg::StepLimit { limit: 1_048_576 },
                "molangx: evaluation stopped after its budget of 1048576 steps",
            ),
            (
                RuntimeMsg::StructDepthLimit { limit: 32 },
                "molangx: evaluation stopped: a struct would nest deeper than its budget of 32 levels",
            ),
            (
                RuntimeMsg::OperandStackLimit { limit: 65_536 },
                "molangx: evaluation stopped: operands left behind by break / continue passed the cap of 65536",
            ),
            (
                RuntimeMsg::StructMemberLimit { limit: 256 },
                "molangx: evaluation stopped: a struct would hold more than its budget of 256 members",
            ),
            (
                RuntimeMsg::QueryDepthLimit { limit: 8 },
                "molangx: evaluation stopped: query arguments would nest deeper than their budget of 8 levels",
            ),
        ];
        for (msg, text) in cases {
            assert_eq!(msg.to_string(), text);
            assert_eq!(msg.level(), LogLevel::Warn);
        }
    }

    #[test]
    fn the_limits_in_the_messages_print_in_full() {
        assert_eq!(
            RuntimeMsg::StepLimit { limit: u64::MAX }.to_string(),
            "molangx: evaluation stopped after its budget of 18446744073709551615 steps"
        );
        assert_eq!(
            RuntimeMsg::LoopLimit { limit: 0 }.to_string(),
            "molangx: loop stopped after its budget of 0 iterations"
        );
    }

    #[test]
    fn runtime_messages_compare_and_hash_by_content() {
        use std::collections::HashSet;
        assert_eq!(unknown("a"), unknown("a"));
        assert_ne!(unknown("a"), unknown("b"));
        assert_ne!(
            unknown("a"),
            RuntimeMsg::UnknownVariable {
                name: "a",
                public_access: true
            }
        );
        assert_ne!(
            RuntimeMsg::MissingMember { name: "a" },
            RuntimeMsg::IncompatibleType { name: Some("a") }
        );
        let copy = unknown("a");
        assert_eq!(copy, unknown("a"));
        let set: HashSet<RuntimeMsg<'_>> = [
            unknown("a"),
            unknown("a"),
            unknown("b"),
            RuntimeMsg::PublicAccessUnderflow,
        ]
        .into_iter()
        .collect();
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn sinks() {
        let mut collect = CollectSink::new();
        {
            let mut by_ref: &mut dyn RuntimeSink = &mut collect;
            by_ref.runtime(RuntimeMsg::PublicAccessUnderflow);
            (&mut by_ref).query_error(QueryError::new(
                query::MAX_DURABILITY,
                "query.max_durability has no item",
            ));
        }
        assert_eq!(
            collect.messages,
            [
                "molangx: a public-access scope was closed while none was open",
                "query.max_durability has no item"
            ]
        );
        let mut null = NullSink;
        null.runtime(RuntimeMsg::PublicAccessUnderflow);
        null.query_error(QueryError::new(query::MAX_DURABILITY, "x"));
    }

    #[test]
    fn a_mutable_reference_to_a_sink_is_a_sink_that_forwards() {
        fn report<S: RuntimeSink>(mut sink: S) {
            sink.runtime(unknown("variable.a"));
            sink.query_error(error("from a query"));
        }
        let mut collect = CollectSink::new();
        report(&mut collect);
        report(&mut collect);
        assert_eq!(collect.messages.len(), 4);
        assert_eq!(collect.messages[1], "from a query");
        let mut bounded = BoundedSink::new();
        report(&mut bounded as &mut dyn RuntimeSink);
        report(&mut &mut bounded);
        assert_eq!(bounded.messages().len(), 4);
    }

    #[test]
    fn the_null_sink_drops_everything_and_is_a_unit_value() {
        let mut null = NullSink;
        null.runtime(unknown("variable.a"));
        null.query_error(error("x"));
        let fresh = NullSink;
        assert_eq!(null, fresh);
        assert_eq!(size_of::<NullSink>(), 0);
        let mut collect = CollectSink::new();
        collect.runtime(unknown("variable.a"));
        collect.query_error(error("x"));
        assert_ne!(collect, CollectSink::new());
    }

    #[test]
    fn collect_sink_keeps_every_message_whole_and_in_order() {
        let mut sink = CollectSink::new();
        assert!(sink.is_empty());
        for i in 0..5000 {
            sink.runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(sink.messages.len(), 5000);
        assert_eq!(
            sink.messages[0],
            "molangx: loop stopped after its budget of 0 iterations"
        );
        assert_eq!(
            sink.messages[4999],
            "molangx: loop stopped after its budget of 4999 iterations"
        );
        let long = "a".repeat(65_536);
        let mut sink = CollectSink::default();
        sink.runtime(unknown(&long));
        assert_eq!(sink.messages[0].len(), 48 + 65_536);
        assert!(!sink.messages[0].ends_with('…'));
    }

    #[test]
    fn collect_sink_take_empties_it_and_new_equals_default() {
        let mut sink = CollectSink::new();
        sink.runtime(RuntimeMsg::PublicAccessUnderflow);
        sink.query_error(error("e"));
        assert!(!sink.is_empty());
        let taken = sink.take();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[1], "e");
        assert!(sink.is_empty());
        assert!(sink.take().is_empty());
        assert_eq!(CollectSink::new(), CollectSink::default());
    }

    #[test]
    fn bounded_sink_defaults() {
        assert_eq!(BoundedSink::DEFAULT_CAPACITY, 64);
        assert_eq!(BoundedSink::MAX_MESSAGE_BYTES, 1024);
        let sink = BoundedSink::new();
        assert_eq!(sink, BoundedSink::default());
        assert!(sink.is_empty());
        assert_eq!(sink.dropped(), 0);
        assert!(sink.messages().is_empty());
    }

    #[test]
    fn bounded_sink_keeps_the_last_messages_oldest_first() {
        let mut sink = BoundedSink::with_capacity(2);
        for name in ["variable.a", "variable.b", "variable.c"] {
            sink.runtime(unknown(name));
        }
        assert_eq!(sink.dropped(), 1);
        assert_eq!(sink.messages().len(), 2);
        let taken = sink.take();
        assert_eq!(
            taken,
            [
                "Error: unhandled request for unknown variable 'variable.b'",
                "Error: unhandled request for unknown variable 'variable.c'"
            ]
        );
        assert!(sink.is_empty());
        assert_eq!(sink.dropped(), 0);
        sink.runtime(unknown("variable.d"));
        assert_eq!(sink.messages().len(), 1);
        assert_eq!(sink.dropped(), 0);
    }

    #[test]
    fn bounded_sink_with_capacity_zero_only_counts() {
        let mut sink = BoundedSink::with_capacity(0);
        for i in 0..3 {
            sink.runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        sink.query_error(error("e"));
        assert!(sink.is_empty());
        assert_eq!(sink.dropped(), 4);
        assert!(sink.take().is_empty());
        assert_eq!(sink.dropped(), 0);
    }

    #[test]
    fn bounded_sink_default_capacity_keeps_the_last_64() {
        let mut sink = BoundedSink::new();
        for i in 0..100 {
            sink.runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(sink.messages().len(), 64);
        assert_eq!(sink.dropped(), 36);
        assert_eq!(
            sink.messages().front().unwrap(),
            "molangx: loop stopped after its budget of 36 iterations"
        );
        assert_eq!(
            sink.messages().back().unwrap(),
            "molangx: loop stopped after its budget of 99 iterations"
        );
    }

    #[test]
    fn bounded_sink_exactly_at_the_capacity_drops_nothing() {
        let mut sink = BoundedSink::with_capacity(3);
        for i in 0..3 {
            sink.runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(sink.dropped(), 0);
        assert_eq!(sink.messages().len(), 3);
        sink.runtime(RuntimeMsg::LoopLimit { limit: 3 });
        assert_eq!(sink.dropped(), 1);
        assert_eq!(sink.messages().len(), 3);
    }

    fn kept(name: &str) -> String {
        let mut sink = BoundedSink::new();
        sink.runtime(unknown(name));
        sink.take().remove(0)
    }

    #[test]
    fn bounded_sink_keeps_a_message_of_exactly_1024_bytes_whole() {
        // 47 bytes of prefix, the name and a closing quote.
        let text = kept(&"a".repeat(976));
        assert_eq!(text.len(), 1024);
        assert!(text.ends_with("a'"));
        assert!(!text.contains('…'));
        assert_eq!(kept(&"a".repeat(975)).len(), 1023);
    }

    #[test]
    fn bounded_sink_cuts_a_longer_message_to_1024_bytes_and_marks_it() {
        let text = kept(&"a".repeat(977));
        assert_eq!(text.len(), 1024 + '…'.len_utf8());
        assert!(text.starts_with("Error: unhandled request for unknown variable 'aaa"));
        assert!(text.ends_with("a…"));
        let text = kept(&"b".repeat(65_536));
        assert_eq!(text.len(), 1027);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn bounded_sink_cuts_on_a_character_boundary() {
        // The cut falls inside the first three-byte character.
        let name = format!("{}€€", "a".repeat(975));
        let text = kept(&name);
        assert_eq!(text.len(), 47 + 975 + '…'.len_utf8());
        assert!(text.ends_with("a…"));
        assert!(!text.contains('€'));
        let text = kept(&"€".repeat(2000));
        assert!(text.ends_with('…'));
        assert!(text.chars().filter(|c| *c == '€').count() * 3 + 47 <= 1024);
        assert_eq!(text.chars().filter(|c| *c == '€').count(), (1024 - 47) / 3);
        let name = format!("{}€{}", "a".repeat(974), "b".repeat(100));
        let text = kept(&name);
        assert!(text.contains('€'));
        assert_eq!(text.len(), 1024 + '…'.len_utf8());
    }

    #[test]
    fn bounded_sink_appends_nothing_after_a_cut() {
        let text = kept(&"a".repeat(2000));
        assert!(text.ends_with('…'));
        let mut sink = BoundedSink::new();
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: &"a".repeat(2000),
            public_access: true,
        });
        let text = sink.take().remove(0);
        assert!(text.ends_with('…'));
        assert!(!text.contains("are you trying"));
        assert_eq!(text.len(), 1027);
    }

    #[test]
    fn bounded_sink_formats_query_errors() {
        let mut sink = BoundedSink::new();
        sink.query_error(QueryError::new(
            query::IS_BABY,
            format_args!("Error: {} failed on {}", query::IS_BABY, 5),
        ));
        assert_eq!(sink.take(), ["Error: query.is_baby failed on 5"]);
        sink.query_error(error(&"x".repeat(2000)));
        assert_eq!(sink.take()[0].len(), 1027);
    }

    #[test]
    fn bounded_sink_clones_and_compares() {
        let mut sink = BoundedSink::with_capacity(2);
        sink.runtime(unknown("variable.a"));
        let copy = sink.clone();
        assert_eq!(copy, sink);
        sink.runtime(unknown("variable.b"));
        assert_ne!(copy, sink);
        let mut a = BoundedSink::with_capacity(1);
        a.runtime(unknown("x"));
        a.runtime(unknown("y"));
        let mut b = BoundedSink::with_capacity(1);
        b.runtime(unknown("y"));
        assert_ne!(a, b);
        assert_eq!(a.messages(), b.messages());
    }

    #[test]
    fn the_capped_buffer_keeps_the_first_1024_bytes() {
        let mut capped = Capped::default();
        capped.write_str(&"a".repeat(1024)).unwrap();
        assert!(!capped.cut);
        assert_eq!(capped.text.len(), 1024);
        capped.write_str("b").unwrap();
        assert!(capped.cut);
        assert_eq!(capped.text.len(), 1024);
        capped.write_str("").unwrap();
        capped.write_str("c").unwrap();
        assert_eq!(capped.text.len(), 1024);
        assert!(capped.text.chars().all(|c| c == 'a'));
    }

    #[test]
    fn the_capped_buffer_drops_a_character_that_straddles_the_limit_whole() {
        let mut capped = Capped::default();
        capped.write_str(&"a".repeat(1023)).unwrap();
        capped.write_str("é").unwrap();
        assert!(capped.cut);
        assert_eq!(capped.text.len(), 1023);
        let mut capped = Capped::default();
        capped.write_str(&"a".repeat(1022)).unwrap();
        capped.write_str("é").unwrap();
        assert!(!capped.cut);
        assert_eq!(capped.text.len(), 1024);
    }

    #[test]
    fn the_capped_buffer_cuts_the_front_of_one_long_piece() {
        let mut capped = Capped::default();
        capped.write_str("head ").unwrap();
        capped.write_str(&"z".repeat(5000)).unwrap();
        assert!(capped.cut);
        assert_eq!(capped.text.len(), 1024);
        assert!(capped.text.starts_with("head zzz"));
    }

    #[test]
    fn log_once_drops_repeats_by_formatted_text() {
        let mut sink = LogOnce::new(CollectSink::new());
        for _ in 0..3 {
            sink.runtime(RuntimeMsg::UnknownVariable {
                name: "variable.x",
                public_access: false,
            });
        }
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.y",
            public_access: false,
        });
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.x",
            public_access: true,
        });
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.y",
            public_access: false,
        });
        assert_eq!(sink.inner().messages.len(), 3);
        assert_eq!(sink.distinct(), 3);
        assert_eq!(
            sink.inner().messages[0],
            "Error: unhandled request for unknown variable 'variable.x'"
        );
        assert_eq!(
            sink.inner().messages[1],
            "Error: unhandled request for unknown variable 'variable.y'"
        );

        let error = |n: i32| {
            QueryError::new(
                query::HAS_ANY_FAMILY,
                format_args!("argument {n} of query.has_any_family is not a string"),
            )
        };
        sink.query_error(error(1));
        sink.query_error(error(1));
        sink.query_error(error(2));
        assert_eq!(sink.inner().messages.len(), 5);

        sink.reset();
        sink.runtime(RuntimeMsg::UnknownVariable {
            name: "variable.x",
            public_access: false,
        });
        assert_eq!(sink.inner_mut().take().len(), 6);
        assert!(sink.into_inner().is_empty());
    }

    #[test]
    fn log_once_identifies_a_text_not_the_kind_of_message() {
        let mut sink = LogOnce::new(CollectSink::new());
        sink.runtime(RuntimeMsg::PublicAccessUnderflow);
        sink.query_error(error(
            "molangx: a public-access scope was closed while none was open",
        ));
        assert_eq!(sink.inner().messages.len(), 1);
        assert_eq!(sink.distinct(), 1);
        sink.query_error(error("another"));
        sink.runtime(RuntimeMsg::PublicAccessUnderflow);
        assert_eq!(sink.inner().messages.len(), 2);
    }

    #[test]
    fn log_once_lets_distinct_messages_through_in_order() {
        let mut sink = LogOnce::new(CollectSink::new());
        for i in 0..10 {
            sink.runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(sink.distinct(), 10);
        assert_eq!(sink.inner().messages.len(), 10);
        assert_eq!(
            sink.inner().messages[3],
            "molangx: loop stopped after its budget of 3 iterations"
        );
    }

    #[test]
    fn log_once_forwards_a_message_whole_to_a_bounded_sink() {
        let mut sink = LogOnce::new(BoundedSink::new());
        for _ in 0..3 {
            sink.runtime(unknown("variable.x"));
        }
        assert_eq!(sink.inner().messages().len(), 1);
        assert_eq!(sink.into_inner().dropped(), 0);
    }

    #[test]
    fn log_once_seen_cap_wraps_around() {
        assert_eq!(LogOnce::<NullSink>::SEEN_CAP, 4096);
        let names: Vec<String> = (0..4097).map(|i| format!("variable.n{i}")).collect();
        let mut sink = LogOnce::new(CollectSink::new());
        for name in &names[..4096] {
            sink.runtime(unknown(name));
        }
        assert_eq!(sink.distinct(), 4096);
        assert_eq!(sink.inner().messages.len(), 4096);
        sink.runtime(unknown(&names[0]));
        assert_eq!(sink.inner().messages.len(), 4096);
        sink.runtime(unknown(&names[4096]));
        assert_eq!(sink.distinct(), 1);
        assert_eq!(sink.inner().messages.len(), 4097);
        sink.runtime(unknown(&names[0]));
        assert_eq!(sink.inner().messages.len(), 4098);
        assert_eq!(sink.distinct(), 2);
        sink.runtime(unknown(&names[0]));
        assert_eq!(sink.inner().messages.len(), 4098);
    }

    #[test]
    fn log_once_seen_cap_over_a_null_sink_keeps_the_remainder() {
        let mut sink = LogOnce::new(NullSink);
        for i in 0..10_000 {
            sink.runtime(RuntimeMsg::LoopLimit { limit: i });
        }
        assert_eq!(sink.distinct(), 10_000 % 4096);
    }

    #[test]
    fn log_once_reset_starts_a_new_session() {
        let mut sink = LogOnce::new(CollectSink::new());
        sink.runtime(unknown("variable.a"));
        sink.runtime(unknown("variable.a"));
        assert_eq!(sink.distinct(), 1);
        sink.reset();
        assert_eq!(sink.distinct(), 0);
        sink.runtime(unknown("variable.a"));
        assert_eq!(sink.inner().messages.len(), 2);
    }

    #[test]
    fn log_once_accessors_expose_the_inner_sink() {
        let mut sink = LogOnce::new(CollectSink::new());
        sink.runtime(RuntimeMsg::PublicAccessUnderflow);
        assert_eq!(sink.inner().messages.len(), 1);
        assert_eq!(sink.inner_mut().take().len(), 1);
        assert!(sink.inner().is_empty());
        sink.runtime(RuntimeMsg::PublicAccessUnderflow);
        assert!(sink.inner().is_empty());
        assert!(sink.into_inner().is_empty());
    }

    #[test]
    fn log_once_default_is_empty_and_clone_copies_the_seen_set() {
        let sink = LogOnce::<CollectSink>::default();
        assert_eq!(sink.distinct(), 0);
        assert!(sink.inner().is_empty());
        let mut original = LogOnce::new(CollectSink::new());
        original.runtime(unknown("variable.a"));
        let mut copy = original.clone();
        copy.runtime(unknown("variable.a"));
        assert_eq!(copy.inner().messages.len(), 1);
        copy.runtime(unknown("variable.b"));
        assert_eq!(copy.inner().messages.len(), 2);
        assert_eq!(original.inner().messages.len(), 1);
        assert_eq!(original.distinct(), 1);
    }

    #[test]
    fn log_once_passes_an_empty_text_once() {
        let mut sink = LogOnce::new(CollectSink::new());
        sink.query_error(error(""));
        sink.query_error(error(""));
        assert_eq!(sink.inner().messages, [""]);
        assert_eq!(sink.distinct(), 1);
    }

    #[test]
    fn the_fnv_writer_is_a_streaming_fnv1() {
        assert_eq!(FnvWriter::new().finish(), 0);
        let mut empty = FnvWriter::new();
        empty.write_str("").unwrap();
        assert_eq!(empty.finish(), 0);

        let mut a = FnvWriter::new();
        a.write_str("a").unwrap();
        assert_eq!(a.finish(), 12_638_153_115_695_167_422);
        assert_eq!(a.finish(), HashedStr::new("a").as_u64());

        let mut whole = FnvWriter::new();
        whole.write_str("hello world").unwrap();
        let mut pieces = FnvWriter::new();
        for piece in ["hel", "", "lo", " ", "world"] {
            pieces.write_str(piece).unwrap();
        }
        assert_eq!(whole.finish(), pieces.finish());
        assert_eq!(whole.finish(), HashedStr::new("hello world").as_u64());
    }
}
