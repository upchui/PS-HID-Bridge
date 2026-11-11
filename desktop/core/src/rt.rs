//! Real-time scheduling for the threads on the input path.
//!
//! Under CPU load (e.g. video decoding next to us) a normal thread can wait
//! several milliseconds before it runs; a SCHED_FIFO thread runs at once.
//! Two ways to get there, without root:
//! * directly, if RLIMIT_RTPRIO allows it (e.g. the "realtime" group);
//! * through rtkit over D-Bus (feature "rtkit", used by the desktop app),
//!   which desktop systems with PipeWire usually run.
//!
//! RLIMIT_RTTIME is always set first: a real-time thread that computes for
//! longer than that without sleeping gets a signal instead of freezing a CPU
//! core.

use std::io;

/// Priority for the input threads: above normal threads, below audio
/// servers (PipeWire uses 20 by default).
pub const PRIORITY: i32 = 10;

/// Longest CPU time without blocking (µs); rtkit's default maximum.
const RTTIME_US: u64 = 200_000;

/// Make the calling thread real-time. Returns how it worked out.
pub fn make_current_thread_realtime(priority: i32) -> Result<&'static str, String> {
    set_rttime_limit(RTTIME_US).map_err(|e| format!("RLIMIT_RTTIME: {e}"))?;
    let direct = sched_fifo(priority);
    if direct.is_ok() {
        return Ok("SCHED_FIFO");
    }
    #[cfg(feature = "rtkit")]
    {
        match rtkit::make_realtime(priority) {
            Ok(()) => Ok("rtkit"),
            Err(e) => Err(format!("not permitted ({}), rtkit: {e}", direct.unwrap_err())),
        }
    }
    #[cfg(not(feature = "rtkit"))]
    Err(format!("not permitted ({})", direct.unwrap_err()))
}

fn sched_fifo(priority: i32) -> io::Result<()> {
    let param = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: pid 0 = the calling thread; valid sched_param.
    let r = unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO | libc::SCHED_RESET_ON_FORK, &param) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn set_rttime_limit(us: u64) -> io::Result<()> {
    let mut cur = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: valid rlimit pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut cur) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let lim = libc::rlimit {
        rlim_cur: us.min(cur.rlim_max),
        rlim_max: us.min(cur.rlim_max),
    };
    // SAFETY: valid rlimit pointer.
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &lim) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Scheduling class of the calling thread is SCHED_FIFO or SCHED_RR.
pub fn current_thread_is_realtime() -> bool {
    // SAFETY: plain query for the calling thread.
    let policy = unsafe { libc::sched_getscheduler(0) } & !libc::SCHED_RESET_ON_FORK;
    policy == libc::SCHED_FIFO || policy == libc::SCHED_RR
}

#[cfg(feature = "rtkit")]
mod rtkit {
    use zbus::blocking::{Connection, Proxy};

    pub fn make_realtime(priority: i32) -> Result<(), String> {
        let conn = Connection::system().map_err(|e| e.to_string())?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.RealtimeKit1",
            "/org/freedesktop/RealtimeKit1",
            "org.freedesktop.RealtimeKit1",
        )
        .map_err(|e| e.to_string())?;
        let max: i32 = proxy
            .get_property("MaxRealtimePriority")
            .map_err(|e| e.to_string())?;
        // SAFETY: gettid has no preconditions.
        let tid = unsafe { libc::gettid() } as u64;
        let prio = priority.min(max).max(1) as u32;
        proxy
            .call::<_, _, ()>("MakeThreadRealtime", &(tid, prio))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realtime_or_clear_error() {
        // Depends on the machine's limits; it must not panic and must report
        // a state that matches what it says.
        let h = std::thread::spawn(|| {
            let r = make_current_thread_realtime(PRIORITY);
            (r.is_ok(), current_thread_is_realtime())
        });
        let (ok, rt) = h.join().unwrap();
        assert_eq!(ok, rt);
    }
}
