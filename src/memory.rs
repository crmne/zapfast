//! Returning freed heap memory to the operating system.
//!
//! A long session loads and drops a lot of transient data: laid-out text,
//! decoded images, and the rows of every chat the reader opens. glibc keeps
//! the freed arenas mapped, so `RSS` only ever climbs and never comes back
//! down even once the data is gone. Trimming on a slow timer hands those
//! pages back, so the process shrinks after a busy spell instead of holding
//! its high-water mark for good.

use std::time::{Duration, Instant};

/// How often the heap is trimmed. Rare enough that the walk over the arena
/// list stays off the hot path, frequent enough that a burst of activity is
/// followed by a visible drop.
const INTERVAL: Duration = Duration::from_secs(10);

/// Trims the heap when `INTERVAL` has passed since the last call.
///
/// Cheap to call every frame: it only reads a clock until a trim is due.
pub fn trim_periodically() {
    thread_local! {
        static LAST: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    }
    LAST.with(|last| {
        let now = Instant::now();
        let due = last
            .get()
            .is_none_or(|previous| now.duration_since(previous) >= INTERVAL);
        if due {
            last.set(Some(now));
            trim();
        }
    });
}

/// Returns free heap pages to the operating system now.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim() {
    // `malloc_trim(0)` walks the arena free lists and releases the pages at
    // the top of each heap that are no longer in use. Safe to call at any
    // time; it takes glibc's internal lock and returns whether it freed any.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Other platforms release memory through their own allocator, so there is
/// nothing to do here.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_is_safe_to_call_repeatedly() {
        trim();
        trim();
    }
}
