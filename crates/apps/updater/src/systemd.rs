//! The one thing the updater needs from the world that a file cannot tell it: whether systemd has
//! the bot running.
//!
//! A bot stops when systemd stops it and starts when systemd starts it, so both are `systemctl
//! --user` calls on the bot's unit. The policy in `svanbot_runtime::update` never calls them; it
//! asks a [`crate::world::World`] whether the bot has stopped and whether it came back, and this is
//! the world that answers with a real unit on a real box.

use std::io;
use std::process::Command;

/// The systemd user manager's view of one bot, `svanbot@<name>.service`.
pub(crate) struct Systemd {
    unit: String,
}

impl Systemd {
    /// The unit for the bot named `me`.
    pub(crate) fn for_bot(me: &str) -> Self {
        Self {
            unit: format!("svanbot@{me}.service"),
        }
    }

    /// Asks systemd to stop the bot, which asks it to stop between hands.
    pub(crate) fn stop(&self) -> io::Result<()> {
        self.systemctl("stop")
    }

    /// Asks systemd to start the bot again, from the build under `current`.
    pub(crate) fn start(&self) -> io::Result<()> {
        self.systemctl("start")
    }

    /// Whether systemd says the unit is running right now.
    pub(crate) fn is_running(&self) -> io::Result<bool> {
        let output = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", &self.unit])
            .status()?;
        Ok(output.success())
    }

    /// Whether systemd has the unit down, inactive or failed, so that no process of the bot is
    /// running. A systemctl that cannot be run or cannot reach the user manager says nothing on
    /// stdout, which reads as not down: the window then waits for a bot that may be playing.
    pub(crate) fn is_down(&self) -> bool {
        match Command::new("systemctl")
            .args(["--user", "is-active", &self.unit])
            .output()
        {
            Ok(output) => down_in(&String::from_utf8_lossy(&output.stdout)),
            Err(error) => {
                eprintln!(
                    "svanbot-release: systemctl --user is-active {}: {error}",
                    self.unit
                );
                false
            }
        }
    }

    fn systemctl(&self, verb: &str) -> io::Result<()> {
        let output = Command::new("systemctl")
            .args(["--user", verb, &self.unit])
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(format!(
            "systemctl --user {verb} {}: {}",
            self.unit,
            said.trim()
        )))
    }
}

/// Whether `state`, the word `systemctl is-active` prints, is a unit with no process running.
///
/// `is-active` exits non-zero for every state but `active`, so its exit code cannot tell a unit
/// that is down from one between two restarts; the word it prints can.
fn down_in(state: &str) -> bool {
    matches!(state.trim(), "inactive" | "failed")
}

#[cfg(test)]
mod tests {
    use super::down_in;

    #[test]
    fn only_an_inactive_or_failed_unit_is_down() {
        // A unit that is not installed reads `inactive` too, and is as down as one that stopped.
        assert!(down_in("inactive\n"));
        assert!(down_in("failed\n"));
        // Anything else may have a process holding a seat, and the window waits for it: a unit
        // between restarts (`activating`), one stopping, and a systemctl that could not say.
        for state in [
            "active\n",
            "activating\n",
            "deactivating\n",
            "reloading\n",
            "",
        ] {
            assert!(!down_in(state), "{state:?}");
        }
    }
}
