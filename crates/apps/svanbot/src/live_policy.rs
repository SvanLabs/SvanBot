//! The assignment this bot has taken up between hands.

use std::cell::RefCell;

use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_strategy::{params::Params, policy::Policy};

pub struct LivePolicy {
    fleet: Fleet,
    bot: String,
    current: RefCell<Result<Option<Assignment>, String>>,
}

impl LivePolicy {
    pub fn open(fleet: Fleet, bot: &str) -> (Self, Vec<String>) {
        let live = Self {
            fleet,
            bot: bot.into(),
            current: RefCell::new(Ok(None)),
        };
        let notes = live.refresh().into_iter().collect();
        (live, notes)
    }

    /// Take up a changed assignment and report it once.
    pub fn refresh(&self) -> Option<String> {
        let next = self.fleet.assignment(&self.bot);
        let mut current = self.current.borrow_mut();
        if next == *current || (next.is_err() && current.is_err()) {
            return None;
        }
        let note = match &next {
            Ok(Some(assignment)) => {
                format!(
                    "plays policy {}/{}",
                    assignment.kind.name(),
                    assignment.style.name
                )
            }
            Ok(None) => "plays the reference policy".into(),
            Err(error) => format!("assignment refused: {error}; plays the reference policy"),
        };
        *current = next;
        Some(note)
    }

    /// Use the current hand's parameter set with the held assignment or reference.
    pub fn policy(&self, params: Params) -> Policy {
        match *self.current.borrow() {
            Ok(Some(assignment)) => Policy::new(assignment.kind, assignment.style, params),
            _ => Policy::reference(params),
        }
    }
}
