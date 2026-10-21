//! A Tokio runtime context for the UI thread.
//!
//! Slint's winit backend reads the desktop colour scheme from the XDG settings portal over
//! D-Bus, on the UI thread, the first time anything on screen depends on it: a text selection,
//! a `Text` with no explicit colour, a `Palette` lookup. This workspace builds zbus with its
//! `tokio` feature, so that code calls `tokio::task::spawn_blocking` and opens its socket through
//! Tokio. On a thread with no runtime context both panic with "there is no reactor running",
//! and with `panic = "abort"` the whole app dies. Selecting text in a Settings text box did.
//!
//! [`enter_ui_thread`] gives the calling thread a runtime context for the rest of the process.
//! The runtime has one worker thread, which drives I/O and timers while the UI thread polls
//! Slint's own executor.

use std::sync::OnceLock;

use slint::PlatformError;
use tokio::runtime::{Builder, Handle, Runtime};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Make `tokio::spawn`, `spawn_blocking` and Tokio I/O work on the calling thread.
///
/// Call it on the thread that runs the Slint event loop. Calling it again, or on a thread
/// that already has a runtime context, does nothing.
pub(crate) fn enter_ui_thread() -> Result<(), PlatformError> {
    if Handle::try_current().is_ok() {
        return Ok(());
    }
    if RUNTIME.get().is_none() {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("smpl-runtime")
            .enable_all()
            .build()
            .map_err(|e| PlatformError::Other(format!("cannot start the async runtime: {e}")))?;
        // Losing a race with another thread only drops the spare runtime.
        let _ = RUNTIME.set(runtime);
    }
    let runtime = RUNTIME.get().expect("the runtime was stored above");
    // The guard must outlive the event loop, so it is never dropped.
    std::mem::forget(runtime.enter());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_thread_has_no_runtime_until_it_enters_one() {
        std::thread::spawn(|| {
            assert!(Handle::try_current().is_err());
            enter_ui_thread().expect("the runtime starts");
            assert!(Handle::try_current().is_ok());
        })
        .join()
        .expect("the thread did not panic");
    }

    #[test]
    fn blocking_work_can_be_spawned_from_the_entered_thread() {
        std::thread::spawn(|| {
            enter_ui_thread().expect("the runtime starts");
            // The call zbus makes first when it connects.
            let task = tokio::task::spawn_blocking(|| 7);
            let runtime = RUNTIME.get().expect("entered above");
            assert_eq!(runtime.block_on(task).expect("the task finished"), 7);
        })
        .join()
        .expect("the thread did not panic");
    }

    #[test]
    fn entering_twice_is_harmless() {
        std::thread::spawn(|| {
            enter_ui_thread().expect("first call");
            enter_ui_thread().expect("second call");
            assert!(Handle::try_current().is_ok());
        })
        .join()
        .expect("the thread did not panic");
    }

    // Slint's colour-scheme watcher begins with `zbus::Connection::session()`, polled on the UI
    // thread by Slint's own executor; a plain `futures` block_on polls it the same way. Nothing
    // listens on this address, so the only acceptable outcome is an error, not a panic.
    #[test]
    fn zbus_can_connect_from_the_entered_thread() {
        std::thread::spawn(|| {
            enter_ui_thread().expect("the runtime starts");
            let connect = async {
                zbus::connection::Builder::address("unix:path=/nonexistent/smpl-common/bus")?
                    .build()
                    .await
            };
            let outcome = futures::executor::block_on(connect);
            assert!(outcome.is_err(), "nothing listens on that address");
        })
        .join()
        .expect("zbus did not panic");
    }

    // The entered context must not stop other code from blocking on a runtime of its own:
    // zbus' blocking API (the Settings keypad, the sync-center GUI) drives a private runtime,
    // and the hints daemon builds a current-thread one.
    #[test]
    fn other_runtimes_can_still_block_on_the_entered_thread() {
        std::thread::spawn(|| {
            enter_ui_thread().expect("the runtime starts");
            let refused =
                zbus::blocking::connection::Builder::address("unix:path=/nonexistent/smpl-common/bus")
                    .and_then(|builder| builder.build());
            assert!(refused.is_err(), "nothing listens on that address");

            let own = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime of its own");
            assert_eq!(own.block_on(async { 6 * 7 }), 42);
        })
        .join()
        .expect("neither runtime panicked");
    }

    #[test]
    fn every_thread_shares_one_runtime() {
        let first = std::thread::spawn(|| {
            enter_ui_thread().expect("the runtime starts");
            Handle::current().id()
        })
        .join()
        .expect("first thread");
        let second = std::thread::spawn(|| {
            enter_ui_thread().expect("the runtime starts");
            Handle::current().id()
        })
        .join()
        .expect("second thread");
        assert_eq!(first, second);
    }
}
