//! Process-level calls the installer needs and std does not offer: asking about a lock a parent
//! process handed down as a bare descriptor, and dying without any cleanup.

use std::os::fd::RawFd;

/// Whether this process holds, or can take, the exclusive `flock(2)` lock on descriptor `fd`
/// without waiting. A descriptor inherited from the parent shares its open file description, so a
/// lock the parent holds counts as held here; an unrelated holder makes this `false`. A bad
/// descriptor is an error.
pub fn flock_held(fd: RawFd) -> std::io::Result<bool> {
    // SAFETY: flock only reads the descriptor number; a closed or foreign number is EBADF, which
    // is returned rather than acted on. The lock is on the open file description, not on memory.
    let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::WouldBlock { Ok(false) } else { Err(err) }
}

/// End this process with an uncatchable `SIGKILL`: no destructor, no `atexit`, no flush. A test of
/// crash recovery needs exactly that, since a clean exit would run the cleanup it is probing.
pub fn kill_self() -> ! {
    // SAFETY: kill(2) on our own pid with SIGKILL; it does not return.
    unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
    std::process::abort()
}
