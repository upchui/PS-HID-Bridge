//! Monotonic time in microseconds.

fn read(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}

/// CLOCK_MONOTONIC: the clock of the kernel timestamps on input events (the
/// input devices are switched to it), so the time from an event to its
/// packet can be measured.
pub fn now_us() -> u64 {
    read(libc::CLOCK_MONOTONIC)
}

/// CLOCK_MONOTONIC_RAW: the hardware clock as it runs. NTP changes
/// CLOCK_MONOTONIC's rate (systemd-timesyncd by tens of ppm, while it slews
/// an offset by up to 500 ppm); compared with another clock -- the board's,
/// for the one-way time (oneway.rs) -- that bends the offset between them.
pub fn raw_us() -> u64 {
    read(libc::CLOCK_MONOTONIC_RAW)
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
        let a = raw_us();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = raw_us();
        assert!(b >= a + 2_000);
    }
}
