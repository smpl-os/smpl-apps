//! POSIX signal delivery for `start-menu --resident`.
//!
//! The handled signals are blocked in every thread and consumed synchronously
//! by one waiting thread, so no async-signal handler runs and no other thread
//! is interrupted. Requests are then executed on the Slint event loop.
//! Rust's `Command` clears the blocked mask in launched children.

use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// SIGUSR1: show if hidden, hide if shown.
    Toggle,
    /// SIGRTMIN: show (single-instance handoff); never hides.
    Show,
    /// SIGUSR2: hide (click-outside); never shows.
    Hide,
    /// SIGTERM / SIGINT: clean exit.
    Quit,
    /// SIGCHLD: reap launched children.
    Reap,
}

pub fn show_signal() -> libc::c_int {
    libc::SIGRTMIN()
}

fn handled() -> [libc::c_int; 6] {
    [
        libc::SIGUSR1,
        libc::SIGUSR2,
        libc::SIGTERM,
        libc::SIGINT,
        libc::SIGCHLD,
        show_signal(),
    ]
}

pub fn request_for(signal: libc::c_int) -> Option<Request> {
    match signal {
        libc::SIGUSR1 => Some(Request::Toggle),
        libc::SIGUSR2 => Some(Request::Hide),
        libc::SIGTERM | libc::SIGINT => Some(Request::Quit),
        libc::SIGCHLD => Some(Request::Reap),
        signal if signal == show_signal() => Some(Request::Show),
        _ => None,
    }
}

pub struct Blocked(libc::sigset_t);

/// Block the handled signals in the calling thread, and therefore in every
/// thread it creates afterwards. Call before any thread is spawned.
pub fn block() -> io::Result<Blocked> {
    // SAFETY: the set is initialised by sigemptyset before any other use.
    unsafe {
        let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        let mut set = set.assume_init();
        for signal in handled() {
            libc::sigaddset(&mut set, signal);
        }
        match libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) {
            0 => Ok(Blocked(set)),
            error => Err(io::Error::from_raw_os_error(error)),
        }
    }
}

/// Wait for the blocked signals on a dedicated thread. `deliver` runs on that
/// thread and must forward the request (e.g. `slint::invoke_from_event_loop`).
pub fn listen(
    blocked: Blocked,
    deliver: impl Fn(libc::c_int, Request) + Send + 'static,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || loop {
            match wait(&blocked) {
                Ok(signal) => {
                    if let Some(request) = request_for(signal) {
                        deliver(signal, request);
                    }
                }
                Err(error) => {
                    eprintln!("start-menu: waiting for signals failed: {error}");
                    return;
                }
            }
        })?;
    Ok(())
}

fn wait(blocked: &Blocked) -> io::Result<libc::c_int> {
    loop {
        // SAFETY: `blocked.0` is an initialised signal set; info may be null.
        let signal = unsafe { libc::sigwaitinfo(&blocked.0, std::ptr::null_mut()) };
        if signal >= 0 {
            return Ok(signal);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_map_to_resident_requests() {
        assert_eq!(request_for(libc::SIGUSR1), Some(Request::Toggle));
        assert_eq!(request_for(libc::SIGUSR2), Some(Request::Hide));
        assert_eq!(request_for(libc::SIGRTMIN()), Some(Request::Show));
        assert_eq!(request_for(libc::SIGTERM), Some(Request::Quit));
        assert_eq!(request_for(libc::SIGINT), Some(Request::Quit));
        assert_eq!(request_for(libc::SIGCHLD), Some(Request::Reap));
        for other in [libc::SIGHUP, libc::SIGPIPE, libc::SIGRTMIN() + 1] {
            assert_eq!(request_for(other), None, "signal {other}");
        }
    }

    #[test]
    fn blocked_signals_are_waited_for_instead_of_delivered() {
        // A fresh thread, so the test harness threads keep their masks.
        std::thread::spawn(|| {
            let blocked = block().unwrap();
            // SAFETY: querying the current mask into an initialised set.
            let current = unsafe {
                let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
                libc::sigemptyset(set.as_mut_ptr());
                let mut set = set.assume_init();
                assert_eq!(
                    libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut set),
                    0
                );
                set
            };
            for signal in handled() {
                // SAFETY: both sets are initialised.
                unsafe {
                    assert_eq!(libc::sigismember(&current, signal), 1, "signal {signal}");
                    assert_eq!(libc::sigismember(&blocked.0, signal), 1, "signal {signal}");
                }
            }
            // Thread-directed and blocked, so it stays pending for this thread.
            // SAFETY: SIGUSR2 is blocked in this thread (asserted above).
            assert_eq!(unsafe { libc::raise(libc::SIGUSR2) }, 0);
            assert_eq!(wait(&blocked).unwrap(), libc::SIGUSR2);
        })
        .join()
        .unwrap();
    }
}
