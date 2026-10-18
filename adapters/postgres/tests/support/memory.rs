//! Allocation accounting shared by native read and snapshot laws.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Allocator;

pub static LIVE: AtomicUsize = AtomicUsize::new(0);
pub static PEAK: AtomicUsize = AtomicUsize::new(0);
pub static TOTAL: AtomicUsize = AtomicUsize::new(0);

// Counters publish no object ownership or contents; only allocation sizes.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: System receives the unchanged allocator layout contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let before = LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            PEAK.fetch_max(before.saturating_add(layout.size()), Ordering::Relaxed);
            TOTAL.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: pointer/layout come from the matching System allocation.
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
