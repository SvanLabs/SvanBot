//! The one thing the updater needs from the world that a file cannot tell it: whether the bot's
//! supervisor has it running.
//!
//! A bot runs under `svanbot-bot` (`scripts/bot.sh`, V11-52), so stopping it, starting it and
//! asking after it are calls of that program. The policy in `svanbot_runtime::update` never makes
//! them; it asks a [`crate::world::BotWorld`] whether the bot has stopped and whether it came back,
//! and this is what answers with a real process on a real box.

use std::io;
use std::process::Command;

/// The supervisor's view of one bot.
pub(crate) struct Supervisor {
    program: String,
    bot: String,
}

impl Supervisor {
    /// The supervisor of the bot named `me`: `svanbot-bot` on the PATH, or `SVANBOT_BOT`.
    pub(crate) fn for_bot(me: &str) -> Self {
        Self {
            program: std::env::var("SVANBOT_BOT").unwrap_or_else(|_| "svanbot-bot".into()),
            bot: me.into(),
        }
    }

    /// Stops the bot and returns once it has exited.
    pub(crate) fn stop(&self) -> io::Result<()> {
        self.call("stop")
    }

    /// Starts the bot again, from the build under `current`.
    pub(crate) fn start(&self) -> io::Result<()> {
        self.call("start")
    }

    /// Whether the supervisor says the bot is running right now.
    pub(crate) fn is_running(&self) -> io::Result<bool> {
        let status = Command::new(&self.program)
            .args(["status", &self.bot])
            .output()?
            .status;
        Ok(status.success())
    }

    /// Whether the bot is down, stopped or failed, so that no process of it is running. A
    /// supervisor that cannot be run says nothing on stdout, which reads as not down: the window
    /// then waits for a bot that may be playing.
    pub(crate) fn is_down(&self) -> bool {
        match Command::new(&self.program)
            .args(["status", &self.bot])
            .output()
        {
            Ok(output) => down_in(&String::from_utf8_lossy(&output.stdout)),
            Err(error) => {
                eprintln!(
                    "svanbot-release: {} status {}: {error}",
                    self.program, self.bot
                );
                false
            }
        }
    }

    fn call(&self, verb: &str) -> io::Result<()> {
        let output = Command::new(&self.program)
            .args([verb, &self.bot])
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(format!(
            "{} {verb} {}: {}",
            self.program,
            self.bot,
            said.trim()
        )))
    }
}

/// Whether `state`, the word `svanbot-bot status` prints, is a bot with no process running.
///
/// `status` exits non-zero for every state but `running`, so its exit code cannot tell a bot that
/// is down from one between two restarts; the word it prints can.
fn down_in(state: &str) -> bool {
    matches!(state.trim(), "stopped" | "failed")
}

#[cfg(test)]
mod tests {
    use super::down_in;

    #[test]
    fn only_a_stopped_or_failed_bot_is_down() {
        // A bot that was never started reads `stopped` too, and is as down as one that stopped.
        assert!(down_in("stopped\n"));
        assert!(down_in("failed\n"));
        // Anything else may have a process holding a seat, and the window waits for it: a bot
        // between restarts, one running, and a supervisor that could not say.
        for state in ["running\n", "restarting\n", ""] {
            assert!(!down_in(state), "{state:?}");
        }
    }
}
