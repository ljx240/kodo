//! Ctrl-C handling: the SIGINT handler only flips a flag (async-signal-safe);
//! the turn's `Alive`/`Approve`/`Emit` callbacks observe it and stop.
//!
//! The handler is installed **without** SA_RESTART so a blocking stdin read
//! surfaces `ErrorKind::Interrupted` at the REPL prompt.

use std::sync::atomic::{AtomicBool, Ordering};

static PENDING: AtomicBool = AtomicBool::new(false);

pub fn pending() -> bool {
    PENDING.load(Ordering::SeqCst)
}

pub fn set(value: bool) {
    PENDING.store(value, Ordering::SeqCst);
}

/// Clears a pending interrupt; returns whether one was pending.
pub fn take() -> bool {
    PENDING.swap(false, Ordering::SeqCst)
}

#[cfg(unix)]
extern "C" fn on_sigint(_signal: libc::c_int) {
    PENDING.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
pub fn install() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sigint as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        // No SA_RESTART: a Ctrl-C during a blocking read must wake it.
        action.sa_flags = 0;
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }
}

#[cfg(not(unix))]
pub fn install() {}
