//! The send limiter: the venue drops a message past 20 a second on one connection
//! (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "Authentication" and "Error Codes").
// `Instant` has no constructor but `now`. Each test reads it once as a starting point and never
// waits on the clock: the limiter is handed every moment it sees.
#![allow(clippy::disallowed_methods)]

use std::time::{Duration, Instant};

use svanbot_runtime::limit::Limiter;

#[test]
fn a_burst_up_to_the_limit_goes_at_once_and_the_next_waits_out_the_window() {
    let mut limiter = Limiter::new(3, Duration::from_secs(1));
    let t0 = Instant::now();
    for _ in 0..3 {
        assert_eq!(limiter.admit(t0), Duration::ZERO);
    }
    assert_eq!(
        limiter.admit(t0 + Duration::from_millis(400)),
        Duration::from_millis(600)
    );
}

#[test]
fn a_message_that_waited_counts_from_when_it_went() {
    let mut limiter = Limiter::new(1, Duration::from_secs(1));
    let t0 = Instant::now();
    limiter.admit(t0);
    assert_eq!(limiter.admit(t0), Duration::from_secs(1));
    // The second one went at t0 + 1 s, so a third at that moment waits a full window.
    assert_eq!(
        limiter.admit(t0 + Duration::from_secs(1)),
        Duration::from_secs(1)
    );
}

#[test]
fn old_sends_roll_out_of_the_window() {
    let mut limiter = Limiter::new(2, Duration::from_secs(1));
    let t0 = Instant::now();
    limiter.admit(t0);
    limiter.admit(t0 + Duration::from_millis(500));
    assert_eq!(
        limiter.admit(t0 + Duration::from_millis(1000)),
        Duration::ZERO
    );
}

#[test]
fn the_venues_limiter_stays_under_twenty_a_second() {
    let mut limiter = Limiter::venue();
    let t0 = Instant::now();
    let mut at = t0;
    let mut sent = Vec::new();
    for _ in 0..60 {
        at += limiter.admit(at);
        sent.push(at);
    }
    for (i, start) in sent.iter().enumerate() {
        let in_a_second = sent[i..]
            .iter()
            .take_while(|&&t| t < *start + Duration::from_secs(1))
            .count();
        assert!(in_a_second <= 20, "{in_a_second} sends in one second");
    }
}
