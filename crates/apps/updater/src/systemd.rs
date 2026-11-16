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
