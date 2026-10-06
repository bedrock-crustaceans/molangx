//! Memory and time bounds of one compile of a hostile source up to [`MAX_SOURCE_LEN`] bytes.

#![cfg(all(feature = "compiler", feature = "stdlib"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use molangx::compile::{CompileOptions, Compiled, Deviations, MAX_SOURCE_LEN, compile};
use molangx::diag::LanguageMessage;
use molangx::version::MolangVersion;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` with the same arguments.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let current = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(current, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            let grown = new_size - layout.size();
            let current = CURRENT.fetch_add(grown, Ordering::Relaxed) + grown;
            PEAK.fetch_max(current, Ordering::Relaxed);
        } else {
            CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The counters are process-wide: one measurement at a time.
static SERIAL: Mutex<()> = Mutex::new(());

struct Cost {
    compiled: Compiled,
    /// Bytes live at the compile's peak, above those live before it.
    peak: usize,
    kept: usize,
    elapsed: Duration,
}

fn measure(src: &str, opts: &CompileOptions) -> Cost {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let start = Instant::now();
    let compiled = compile(src, opts);
    let elapsed = start.elapsed();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    let kept = CURRENT.load(Ordering::Relaxed).saturating_sub(before);
    Cost {
        compiled,
        peak,
        kept,
        elapsed,
    }
}

const MIB: usize = 1 << 20;

/// 21,845 bad exponents in 65,535 bytes, each message quoting the rest of the input: formatted
/// eagerly they would take 717 MB.
#[test]
fn bad_exponents_share_one_copy_of_the_source() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let src = "1e;".repeat(21_845);
    assert!(src.len() <= MAX_SOURCE_LEN);
    for deviations in [Deviations::ALL, Deviations::NONE] {
        let opts = CompileOptions {
            deviations,
            ..CompileOptions::server(MolangVersion::LATEST)
        };
        let cost = measure(&src, &opts);
        let compiled = &cost.compiled;
        assert_eq!(compiled.failure(), None, "{deviations:?}");
        let bad: Vec<_> = compiled
            .diagnostics()
            .iter()
            .filter(|d| d.language_message() == Some(LanguageMessage::BadExponent))
            .collect();
        assert!(!bad.is_empty());
        assert_eq!(
            bad[0].message(),
            format!(
                "error parsing float string, expected '+' or '-' after 'e': {}",
                &src[2..]
            )
        );
        assert_eq!(
            bad[1].message(),
            format!(
                "error parsing float string, expected '+' or '-' after 'e': {}",
                &src[5..]
            )
        );
        // The peak is the parser's working set for 43,690 tokens; the diagnostics add a few
        // hundred KiB at most.
        assert!(
            cost.peak < 24 * MIB,
            "{deviations:?}: peak {} bytes",
            cost.peak
        );
        assert!(
            cost.kept < 4 * MIB,
            "{deviations:?}: kept {} bytes",
            cost.kept
        );
        assert!(
            cost.elapsed < Duration::from_secs(5),
            "{deviations:?}: {:?}",
            cost.elapsed
        );
        eprintln!(
            "{deviations:?}: {} diagnostics, peak {} KiB, kept {} KiB, {:?}",
            compiled.diagnostics().len(),
            cost.peak / 1024,
            cost.kept / 1024,
            cost.elapsed
        );
    }
}

/// The other messages that quote the input are logged at most once per compile.
#[test]
fn messages_that_quote_the_source_stay_linear() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let opts = CompileOptions::server(MolangVersion::LATEST);
    let len = MAX_SOURCE_LEN;
    let cases = [
        (
            "unknown token",
            format!("1+{}", "a".repeat(len - 2)),
            LanguageMessage::UnknownToken,
        ),
        (
            "unrecognized token",
            format!("q.{}", "x".repeat(len - 2)),
            LanguageMessage::UnrecognizedToken,
        ),
        (
            "multiple roots",
            "1 ".repeat(len / 2),
            LanguageMessage::MultipleRoots,
        ),
        // 3.4e38 prints as 39 digits and six decimals: the longest token-list line per source byte.
        (
            "multiple roots, long floats",
            "3e38 ".repeat(len / 5),
            LanguageMessage::MultipleRoots,
        ),
    ];
    for (label, src, expected) in cases {
        let cost = measure(&src, &opts);
        assert!(
            cost.compiled
                .diagnostics()
                .iter()
                .any(|d| d.language_message() == Some(expected)),
            "{label}: {:?}",
            cost.compiled.diagnostics().first()
        );
        assert!(cost.compiled.diagnostics().len() < 8, "{label}");
        assert!(cost.peak < 24 * MIB, "{label}: peak {} bytes", cost.peak);
        assert!(cost.kept < 8 * MIB, "{label}: kept {} bytes", cost.kept);
        assert!(
            cost.elapsed < Duration::from_secs(5),
            "{label}: {:?}",
            cost.elapsed
        );
    }
}
