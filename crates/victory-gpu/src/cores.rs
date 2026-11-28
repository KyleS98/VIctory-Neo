//! Which processors our own threads run on, where a processor has two kinds
//! of cores. `cpu_sets` and `keep_on_fast_cores` are fxceless/victory-fork's
//! (`framelog.rs`, commit 0b0074e0, GPL-3.0-or-later, as this).

/// The system's processor sets of group 0: each one's id and its efficiency
/// class (higher is faster).
fn cpu_sets() -> Vec<(u32, u8)> {
    use windows_sys::Win32::System::SystemInformation::{GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut bytes = 0u32;
    // SAFETY: a size query, then the call into a buffer of that size.
    unsafe { GetSystemCpuSetInformation(std::ptr::null_mut(), 0, &mut bytes, GetCurrentProcess(), 0) };
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8).max(1)];
    // SAFETY: as above; the buffer holds `bytes` bytes, aligned for the records.
    let ok = unsafe { GetSystemCpuSetInformation(buffer.as_mut_ptr().cast(), bytes, &mut bytes, GetCurrentProcess(), 0) };
    let mut sets = Vec::new();
    let mut at = 0usize;
    while ok != 0 && at + size_of::<SYSTEM_CPU_SET_INFORMATION>() <= bytes as usize {
        // SAFETY: a record inside the buffer; each says its own size.
        let info = unsafe { &*(buffer.as_ptr().cast::<u8>().add(at).cast::<SYSTEM_CPU_SET_INFORMATION>()) };
        if info.Size == 0 {
            break;
        }
        // SAFETY: the record's type is CpuSet (the only one there is).
        let set = unsafe { &info.Anonymous.CpuSet };
        if set.Group == 0 {
            sets.push((set.Id, set.EfficiencyClass));
        }
        at += info.Size as usize;
    }
    sets
}

/// One of the system's processors (group 0): its set's id, its number, and the
/// last-level cache it shares.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cpu {
    id: u32,
    logical: u8,
    cache: u8,
}

fn cpus() -> Vec<Cpu> {
    use windows_sys::Win32::System::SystemInformation::{GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut bytes = 0u32;
    // SAFETY: a size query, then the call into a buffer of that size.
    unsafe { GetSystemCpuSetInformation(std::ptr::null_mut(), 0, &mut bytes, GetCurrentProcess(), 0) };
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8).max(1)];
    // SAFETY: as above; the buffer holds `bytes` bytes, aligned for the records.
    let ok = unsafe { GetSystemCpuSetInformation(buffer.as_mut_ptr().cast(), bytes, &mut bytes, GetCurrentProcess(), 0) };
    let mut all = Vec::new();
    let mut at = 0usize;
    while ok != 0 && at + size_of::<SYSTEM_CPU_SET_INFORMATION>() <= bytes as usize {
        // SAFETY: a record inside the buffer; each says its own size.
        let info = unsafe { &*(buffer.as_ptr().cast::<u8>().add(at).cast::<SYSTEM_CPU_SET_INFORMATION>()) };
        if info.Size == 0 {
            break;
        }
        // SAFETY: the record's type is CpuSet (the only one there is).
        let set = unsafe { &info.Anonymous.CpuSet };
        if set.Group == 0 {
            all.push(Cpu { id: set.Id, logical: set.LogicalProcessorIndex, cache: set.LastLevelCacheIndex });
        }
        at += info.Size as usize;
    }
    all
}

/// The processors `asked` names among `all`: `<first>-<last>` by their
/// numbers; `auto`, those of every last-level cache but the one processor 0
/// has, if there is more than one (a processor of two dies). `None`: not
/// understood, or none.
fn own_of(asked: &str, all: &[Cpu]) -> Option<Vec<u32>> {
    let asked = asked.trim();
    let chosen: Vec<u32> = if asked.eq_ignore_ascii_case("auto") {
        let first = all.iter().find(|c| c.logical == 0)?.cache;
        all.iter().filter(|c| c.cache != first).map(|c| c.id).collect()
    } else {
        let (from, to) = asked.split_once('-')?;
        let (from, to) = (from.trim().parse::<u8>().ok()?, to.trim().parse::<u8>().ok()?);
        all.iter().filter(|c| (from..=to).contains(&c.logical)).map(|c| c.id).collect()
    };
    (!chosen.is_empty() && chosen.len() < all.len()).then_some(chosen)
}

/// Whether the many threads whose work is ahead of what a frame waits for
/// (lists translated, records made) run on every processor while the others
/// of ours keep to their own ([`keep_own_threads`]; `VICTORY_OWN_CPUS_MANY=1`,
/// off by default: they keep to ours too, see there).
fn roaming() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_OWN_CPUS").is_some() && std::env::var_os("VICTORY_OWN_CPUS_MANY").is_some_and(|v| v == "1"))
}

/// The processors our own threads keep to, as the start names them
/// ([`keep_own_threads`]); `None`: none named.
fn own() -> Option<&'static Vec<u32>> {
    static OWN: std::sync::OnceLock<Option<Vec<u32>>> = std::sync::OnceLock::new();
    OWN.get_or_init(|| {
        let asked = std::env::var("VICTORY_OWN_CPUS").ok()?;
        let all = cpus();
        let own = own_of(&asked, &all);
        match &own {
            Some(own) => tracing::info!(asked, processors = own.len(), of = all.len(), "our own threads are kept on these processors (VICTORY_OWN_CPUS)"),
            None => tracing::warn!(asked, of = all.len(), "VICTORY_OWN_CPUS: no processors of that name, or all of them: left as it is"),
        }
        own
    })
    .as_ref()
}

/// How many threads translate the title's lists where the start does not
/// say (`VICTORY_THIN_THREADS`): eight, or with processors of our own a
/// quarter as many as those, two to eight.
///
/// Eight on sixteen processors of our own, beside the thread that prepares
/// submissions, its eight helpers and the pools, were more threads than those
/// processors run at a time: the usual frame was shorter than with the
/// threads on every processor, and one frame in ten far longer. With four,
/// five and six in their place (the prologue's cell, Ryzen 9 9950X3D, RTX
/// 3090 Ti, Series S profile, DLSS off, medians of five-second readings and
/// the lowest of them): 39.4 and 38.2 frames a second, 38.8 and 37.5, 39.0
/// and 36.4; with eight 38.4 and 34.3; with eight on every processor 37.4 and
/// 36.2. The thread that prepares submissions worked 22.8 ms a frame with
/// four where it worked 24.4.
pub(crate) fn list_threads() -> usize {
    own().map_or(8, |own| (own.len() / 4).clamp(2, 8))
}

/// The processors our own threads run on, where a start names them
/// (`VICTORY_OWN_CPUS=<first>-<last>`, or `auto`: on a processor of two dies,
/// the die the title does not use; not given: any, as before).
///
/// The title gives each of its threads an affinity of its own, to the
/// console's sixteen processors: here the first sixteen, on a Ryzen of two
/// dies one die. Our threads have none and ran on all of them: the thread
/// that prepares submissions half its time on the title's die, beside the
/// title's threads on their cores' second halves, and the other half on the
/// other die, its memory and its locks crossing between the two (a timeline
/// of the prologue's cell, Ryzen 9 9950X3D: the title's threads on processors
/// 0 to 15 alone, the preparing thread 48 % there). Asked for as the
/// process's default sets, the choice holds for every thread that has neither
/// an affinity nor sets of its own: ours, the drivers', and not the title's.
///
/// The many threads that translate lists and make records keep to them too
/// (`VICTORY_OWN_CPUS_MANY=1`: they run on every processor). On the title's
/// die they took turns with the title's main thread on its two processors
/// (half of what ran there in a timeline was ours), and that thread's own
/// work is the longest part of a frame's chain, 22 of 27 ms. Fewer of them
/// are made in their place ([`list_threads`]).
///
/// `VICTORY_OWN_CPUS_ON=0` leaves them as they were (and `VICTORY_AB` can
/// flip it while the title runs).
pub(crate) fn keep_own_threads() {
    use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessDefaultCpuSets};
    let Some(own) = own() else {
        return;
    };
    // 0: not asked yet; 1: kept; 2: let go.
    static APPLIED: AtomicU8 = AtomicU8::new(0);
    let want = if crate::toggled!("VICTORY_OWN_CPUS_ON") { 1 } else { 2 };
    if APPLIED.swap(want, Relaxed) == want {
        return;
    }
    // SAFETY: our own process, and a list of the system's own set ids (or none: the default again).
    let ok = unsafe {
        if want == 1 {
            SetProcessDefaultCpuSets(GetCurrentProcess(), own.as_ptr(), own.len() as u32)
        } else {
            SetProcessDefaultCpuSets(GetCurrentProcess(), std::ptr::null(), 0)
        }
    };
    if ok == 0 {
        tracing::warn!(error = %std::io::Error::last_os_error(), "the process's default processor sets could not be set (VICTORY_OWN_CPUS)");
    }
}

/// The sets of the fastest class, if there are slower ones too.
fn fast_of(sets: &[(u32, u8)]) -> Vec<u32> {
    let best = sets.iter().map(|c| c.1).max().unwrap_or(0);
    let fast: Vec<u32> = sets.iter().filter(|c| c.1 == best).map(|c| c.0).collect();
    if fast.len() == sets.len() { Vec::new() } else { fast }
}

/// `VICTORY_FAST_CORES`: 0: no thread asks; 1: the threads a frame waits for
/// one by one; 2 (the default): the threads they share their work with too;
/// 3: those that work beside the rest as well (lists translated, descriptors
/// made): more threads by then than the fast cores run at a time, while the
/// slow ones stood idle.
fn stage() -> u32 {
    static STAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *STAGE.get_or_init(|| std::env::var("VICTORY_FAST_CORES").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(2))
}

/// Asks the system to run the calling thread, which a frame waits for, on
/// the fast cores only, where the processor has two kinds. That fork's submit
/// thread was found on a slow one in 1,529 looks of 24,021. (A selection of
/// processors is a preference the scheduler keeps unless nothing of it is
/// free.) The title's threads are not asked for.
pub(crate) fn keep_on_fast_cores(what: &str) {
    ask(what, 1);
}

/// [`keep_on_fast_cores`] for one of the threads a submission's preparing or
/// a landing shares its work with.
pub(crate) fn keep_helper_on_fast_cores(what: &str) {
    ask(what, 2);
}

/// [`keep_on_fast_cores`] for one of many threads whose work is ahead of what
/// a frame waits for.
pub(crate) fn keep_many_on_fast_cores(what: &str) {
    ask(what, 3);
}

/// The priority our own threads that a frame waits for, and those they share
/// their work with, run at (`VICTORY_HELPER_PRIORITY=<n>`, a thread priority
/// from -2 to 2; not given: as made, normal, the submitting thread above
/// normal). A frame's timeline in the first gameplay area (run timeline,
/// 2026-11-26) had such threads woken and not running yet for 4-11 ms of its
/// chain: the listing threads up to 5.4 ms at a time, while the title's own
/// workers held every processor.
fn helper_priority() -> Option<i32> {
    static PRIORITY: std::sync::OnceLock<Option<i32>> = std::sync::OnceLock::new();
    *PRIORITY.get_or_init(|| {
        std::env::var("VICTORY_HELPER_PRIORITY").ok().and_then(|v| v.trim().parse::<i32>().ok()).map(|p| p.clamp(-2, 2))
    })
}

fn ask(what: &str, from: u32) {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, SetThreadSelectedCpuSets};
    if let Some(priority) = helper_priority() {
        // SAFETY: this thread's own priority.
        let set = unsafe { SetThreadPriority(GetCurrentThread(), priority) } != 0;
        tracing::debug!(what, priority, set, "the thread's priority (VICTORY_HELPER_PRIORITY)");
    }
    // With processors of our own (`keep_own_threads`) and
    // `VICTORY_OWN_CPUS_MANY=1`: the many threads whose work is ahead of what
    // a frame waits for run on any, the title's too.
    if from == 3 && roaming() {
        static ALL: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
        let all = ALL.get_or_init(|| cpus().iter().map(|c| c.id).collect());
        // SAFETY: our own thread and a list of the system's own set ids.
        let ok = unsafe { SetThreadSelectedCpuSets(GetCurrentThread(), all.as_ptr(), all.len() as u32) };
        tracing::debug!(what, ok, "the thread runs on any processor (VICTORY_OWN_CPUS_MANY)");
    }
    // An experiment's own word on this kind of thread
    // (`VICTORY_THREAD_CPUS=<name>=<first>-<last>;<name>=...`, the name a
    // beginning of the thread's): those processors, whatever the rest says.
    static BY_NAME: std::sync::OnceLock<Vec<(String, Vec<u32>)>> = std::sync::OnceLock::new();
    let by_name = BY_NAME.get_or_init(|| {
        let asked = std::env::var("VICTORY_THREAD_CPUS").unwrap_or_default();
        let all = cpus();
        asked
            .split(';')
            .filter_map(|part| {
                let (name, range) = part.split_once('=')?;
                let own = own_of(range, &all).or_else(|| (range.trim() == "all").then(|| all.iter().map(|c| c.id).collect()))?;
                tracing::info!(name, processors = own.len(), "threads of this name run on these processors (VICTORY_THREAD_CPUS)");
                Some((name.trim().to_owned(), own))
            })
            .collect()
    });
    if let Some((_, own)) = by_name.iter().find(|(name, _)| what.starts_with(name.as_str())) {
        // SAFETY: our own thread and a list of the system's own set ids.
        unsafe { SetThreadSelectedCpuSets(GetCurrentThread(), own.as_ptr(), own.len() as u32) };
        return;
    }
    if stage() < from {
        return;
    }
    static FAST: std::sync::OnceLock<(Vec<u32>, usize)> = std::sync::OnceLock::new();
    let (fast, of) = FAST.get_or_init(|| {
        let sets = cpu_sets();
        (fast_of(&sets), sets.len())
    });
    // All alike (or nothing known): nothing to ask for.
    if fast.is_empty() {
        return;
    }
    // SAFETY: our own thread and a list of the system's own set ids.
    let ok = unsafe { SetThreadSelectedCpuSets(GetCurrentThread(), fast.as_ptr(), fast.len() as u32) };
    if ok == 0 {
        tracing::warn!(what, error = %std::io::Error::last_os_error(), "the thread could not be kept on the fast cores");
    } else {
        tracing::info!(what, fast = fast.len(), of, "the thread is kept on the fast cores (VICTORY_FAST_CORES=0: not)");
    }
}

/// Whether the threads the submitting thread waits for run at its priority
/// (`VICTORY_HELPER_PRIORITY`, on unless `0`; `VICTORY_AB` can flip it while
/// the title runs): above normal, ahead of the title's threads and of the
/// threads that translate lists. They are the threads that spread its work
/// (`shadow::wide`, the pools of `land`), the one that makes the heaps'
/// records and the one that passes over the heap's views beside the lists.
///
/// On a processor with six cores (i5-12400F, 12 threads) the title's threads
/// and eight translating lists kept every core busy, and the submitting
/// thread (above normal since `queue::behind`) stood waiting for helpers at
/// normal priority that were not running. Measured with `VICTORY_AB` in one
/// run (run10, Series S at 540p, medians of 5-second windows in the world,
/// off / on): 8.6 / 9.6 fps, refreshes 23.1 / 17.6 ms a frame, waiting for
/// the heaps' records 6.2 / 2.5 ms, the card idle behind the submitting
/// thread 24.7 / 13.2 ms. Lowering the translating threads below normal as
/// well made each list call about 5 % slower, with no clear gain of its own.
fn helpers_raised() -> bool {
    crate::toggled!("VICTORY_HELPER_PRIORITY")
}

/// A helper's priority ([`helpers_raised`]). [`Self::keep`] is called before
/// each piece of work; it asks the system only when the switch has changed.
pub(crate) struct Priority {
    applied: Option<bool>,
}

impl Priority {
    pub(crate) fn new() -> Self {
        let mut priority = Self { applied: None };
        priority.keep();
        priority
    }

    pub(crate) fn keep(&mut self) {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_NORMAL,
        };
        let on = helpers_raised();
        if self.applied == Some(on) {
            return;
        }
        let priority = if on { THREAD_PRIORITY_ABOVE_NORMAL } else { THREAD_PRIORITY_NORMAL };
        // SAFETY: our own thread.
        unsafe { SetThreadPriority(GetCurrentThread(), priority) };
        self.applied = Some(on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_processors_are_a_range_or_the_other_caches() {
        let all: Vec<Cpu> = (0..8u8).map(|n| Cpu { id: 256 + u32::from(n), logical: n, cache: n / 4 }).collect();
        assert_eq!(own_of("4-7", &all), Some(vec![260, 261, 262, 263]));
        assert_eq!(own_of(" 6 - 9 ", &all), Some(vec![262, 263]));
        assert_eq!(own_of("auto", &all), Some(vec![260, 261, 262, 263]));
        // All of them, none of them, one cache, nonsense: nothing asked for.
        assert_eq!(own_of("0-7", &all), None);
        assert_eq!(own_of("9-12", &all), None);
        let one: Vec<Cpu> = (0..8u8).map(|n| Cpu { id: 256 + u32::from(n), logical: n, cache: 0 }).collect();
        assert_eq!(own_of("auto", &one), None);
        assert_eq!(own_of("fast", &all), None);
        assert!(!cpus().is_empty());
    }

    #[test]
    fn the_fast_cores_are_those_of_the_best_class_if_there_is_a_worse_one() {
        assert_eq!(fast_of(&[(256, 1), (257, 1), (258, 0)]), [256, 257]);
        assert!(fast_of(&[(256, 0), (257, 0)]).is_empty());
        assert!(fast_of(&[]).is_empty());
        assert!(!cpu_sets().is_empty());
        std::thread::spawn(|| keep_on_fast_cores("test")).join().unwrap();
    }
}
