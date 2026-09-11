//! Allocation accounting and peak RSS.
//!
//! Instruction count alone cannot arbitrate an allocation change.
//! This project has measured the boolean path spending about 11%
//! of its instructions in allocation, and separately found a
//! mesh-audit change that cut instructions while making wall time
//! WORSE through cache behaviour. Those two facts together mean a
//! gate that sees only instructions can approve a regression.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// Counting wrapper around the system allocator.
///
/// Relaxed ordering throughout: these are statistics, not
/// synchronisation. Paying for stronger ordering would change the
/// very costs this is measuring.
pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size(), Ordering::Relaxed);
        let live = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}

/// What one measured region allocated.
#[derive(Clone, Copy)]
pub struct Usage {
    /// Number of allocation calls.
    pub allocs: usize,
    /// Total bytes requested, including freed ones.
    pub bytes: usize,
    /// High-water mark of simultaneously live bytes.
    pub peak: usize,
}

/// Measure what `f` allocates.
///
/// Peak is reset to the CURRENT live total rather than to zero, so
/// memory already held on entry is not counted as if this region
/// had allocated it.
pub fn measure<T>(f: impl FnOnce() -> T) -> (T, Usage) {
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let b0 = BYTES.load(Ordering::Relaxed);
    let live0 = LIVE.load(Ordering::Relaxed);
    PEAK.store(live0, Ordering::Relaxed);

    let out = f();

    let usage = Usage {
        allocs: ALLOCS.load(Ordering::Relaxed) - a0,
        bytes: BYTES.load(Ordering::Relaxed) - b0,
        peak: PEAK.load(Ordering::Relaxed).saturating_sub(live0),
    };
    (out, usage)
}

/// Process high-water RSS in kB, from /proc/self/status.
///
/// Complements the counting allocator rather than duplicating it:
/// VmHWM includes pages the allocator never sees -- mmap, stacks,
/// and anything a C++ dependency allocates through its own path.
pub fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A counter that merely returns a plausible number is useless.
    /// This allocates an exactly known amount and checks the report
    /// against it, so the instrument is calibrated before any
    /// conclusion is drawn from what it says about the kernel.
    #[test]
    fn counts_a_known_allocation() {
        let (v, usage) = measure(|| vec![0u8; 1_000_000]);
        assert_eq!(v.len(), 1_000_000);
        assert!(usage.allocs >= 1, "no allocation seen");
        assert!(
            usage.bytes >= 1_000_000,
            "reported {} bytes for a 1 MB vec",
            usage.bytes
        );
        assert!(
            usage.peak >= 1_000_000,
            "peak {} below the live 1 MB",
            usage.peak
        );
    }

    /// Freed memory must not inflate the peak of a later region.
    #[test]
    fn peak_excludes_memory_freed_before_the_region() {
        drop(vec![0u8; 4_000_000]);
        let (_, usage) = measure(|| vec![0u8; 1000]);
        assert!(
            usage.peak < 1_000_000,
            "peak {} carried over a freed 4 MB buffer",
            usage.peak
        );
    }
}
