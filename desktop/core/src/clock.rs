//! Monotonic time in microseconds: the clock of the kernel timestamps on
//! input events (the input devices are switched to CLOCK_MONOTONIC), so the
//! time from an event to its packet can be measured.

pub fn now_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic() {
        let a = now_us();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = now_us();
        assert!(b >= a + 2_000);
    }
}
