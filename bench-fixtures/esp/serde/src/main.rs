//! Bare-metal size harness: decode-only, all three models.
#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

mod canada;
mod citm;
mod twitter;

use core::alloc::{GlobalAlloc, Layout};
use core::hint::black_box;
use core::sync::atomic::{AtomicUsize, Ordering};

/// A bump allocator over a static arena (size harness only).
struct Bump;
static NEXT: AtomicUsize = AtomicUsize::new(0);
static mut ARENA: [u8; 64 * 1024] = [0; 64 * 1024];

unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let start = (NEXT.load(Ordering::Relaxed) + l.align() - 1) & !(l.align() - 1);
        NEXT.store(start + l.size(), Ordering::Relaxed);
        if start + l.size() > 64 * 1024 {
            return core::ptr::null_mut();
        }
        unsafe { (&raw mut ARENA as *mut u8).add(start) }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[global_allocator]
static A: Bump = Bump;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let input: &[u8] = black_box(&[]);
    let a: Result<twitter::Twitter, _> = serde_json::from_slice(input);
    let b: Result<citm::CitmCatalog, _> = serde_json::from_slice(input);
    let c: Result<canada::Canada, _> = serde_json::from_slice(input);
    black_box((a.is_ok(), b.is_ok(), c.is_ok()));
    loop {}
}
