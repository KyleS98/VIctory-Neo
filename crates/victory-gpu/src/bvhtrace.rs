//! `VICTORY_BVH_TRACE=<file>`: what the ray-tracing structure builder is
//! given and what comes back into title memory while it runs, one line
//! each, for finding the stage that stores where it should not.
//!
//! - `D`: a dispatch of a builder program: frame, program, group counts,
//!   root arguments (views with the title buffer they point into).
//! - `W`: bytes put back into title memory from a host copy (what shaders
//!   wrote), from the first builder dispatch on: frame, address, length,
//!   the buffer the address lies in, how many of its dwords are a float
//!   infinity and how many of its words a half-float infinity (an empty
//!   bounding box is +inf, -inf), and its first bytes.
//!
//! Off without the variable: one atomic load at each site.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Option<Mutex<std::io::BufWriter<std::fs::File>>>> = OnceLock::new();
/// A builder dispatch has been traced: writes are listed from here on.
static STARTED: AtomicBool = AtomicBool::new(false);
static LINES: AtomicU64 = AtomicU64::new(0);
/// The most lines written (the run dies within seconds of the first).
const MOST: u64 = 2_000_000;

fn file() -> Option<&'static Mutex<std::io::BufWriter<std::fs::File>>> {
    FILE.get_or_init(|| {
        let path = std::env::var_os("VICTORY_BVH_TRACE")?;
        let file = std::fs::File::create(path).ok()?;
        Some(Mutex::new(std::io::BufWriter::with_capacity(1 << 16, file)))
    })
    .as_ref()
}

pub(crate) fn on() -> bool {
    file().is_some()
}

fn line(text: std::fmt::Arguments) {
    let Some(file) = file() else { return };
    if LINES.fetch_add(1, Ordering::Relaxed) >= MOST {
        return;
    }
    let mut f = file.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writeln!(f, "{}", text);
    // The run ends in a crash: nothing may stay in the buffer.
    let _ = f.flush();
}

thread_local! {
    /// The flags of the `DispatchX` this thread is in, if it is in one.
    static FLAGS: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// The thread enters (`Some`) or leaves a `DispatchX` with these flags.
pub(crate) fn dispatch_flags(flags: Option<u32>) {
    FLAGS.set(flags);
}

/// A dispatch of a builder program.
pub(crate) fn dispatch(text: impl FnOnce() -> String) {
    if !on() {
        return;
    }
    STARTED.store(true, Ordering::Relaxed);
    let flags = match FLAGS.get() {
        Some(f) => format!("DispatchX flags {f:#x}"),
        None => "Dispatch".to_owned(),
    };
    line(format_args!("D\t{}\t{}\t{flags}", crate::frame::current(), text()));
}

/// Any other line (a note of ours).
pub(crate) fn note(text: impl FnOnce() -> String) {
    if on() && STARTED.load(Ordering::Relaxed) {
        line(format_args!("N\t{}\t{}", crate::frame::current(), text()));
    }
}

/// `len` bytes at `src` go into title memory at `at`.
pub(crate) fn wrote(at: usize, src: *const u8, len: usize) {
    if !STARTED.load(Ordering::Relaxed) || len == 0 {
        return;
    }
    // SAFETY: the caller's source of `len` bytes (a host copy or scratch).
    let bytes = unsafe { std::slice::from_raw_parts(src, len) };
    let (mut infs, mut halves) = (0u32, 0u32);
    // By the address's own alignment: the patterns are stored aligned.
    let skew = (4 - at % 4) % 4;
    if bytes.len() > skew {
        for d in bytes[skew..].chunks_exact(4) {
            let v = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
            if v == 0x7f80_0000 || v == 0xff80_0000 {
                infs += 1;
            }
        }
    }
    let skew = at % 2;
    if bytes.len() > skew {
        for d in bytes[skew..].chunks_exact(2) {
            let v = u16::from_le_bytes([d[0], d[1]]);
            if v == 0x7c00 || v == 0xfc00 {
                halves += 1;
            }
        }
    }
    let first: String = bytes.iter().take(24).map(|b| format!("{b:02x}")).collect();
    let inside = match crate::shadow::extent(at as u64) {
        Some((start, n)) => format!("buffer {start:#x}+{n:#x}"),
        None => "no buffer".to_owned(),
    };
    // Where else the page shows (another title view of the same pages).
    let page = at & !0xffff;
    let also: Vec<String> = crate::inplace::shown_at(at)
        .into_iter()
        .filter(|&a| a & !0xffff != page)
        .map(|a| format!("{a:#x}"))
        .collect();
    line(format_args!(
        "W\t{}\t{at:#x}\t{len:#x}\t{inside}\tinf {infs}\thalf {halves}\t{first}\talso {}",
        crate::frame::current(),
        also.join(",")
    ));
}
