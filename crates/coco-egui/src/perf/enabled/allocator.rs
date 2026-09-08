use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use serde_json::{Value, json};

use super::ENABLED;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record(pointer: *mut u8, size: usize) {
    if !pointer.is_null() && ENABLED.load(Relaxed) {
        ALLOCATIONS.fetch_add(1, Relaxed);
        BYTES.fetch_add(size as u64, Relaxed);
    }
}

// SAFETY: Every allocation operation delegates unchanged to System; recording
// uses only atomics and cannot allocate or change the returned pointer.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        record(pointer, layout.size());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record(pointer, layout.size());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        record(pointer, size);
        pointer
    }
}

pub(super) fn reset() {
    ALLOCATIONS.store(0, Relaxed);
    BYTES.store(0, Relaxed);
}

pub(super) fn snapshot() -> Value {
    let allocations = ALLOCATIONS.load(Relaxed);
    let bytes = BYTES.load(Relaxed);
    json!({ "scope": "whole process; successful allocations and reallocations",
        "count": allocations, "requested_bytes": bytes })
}
