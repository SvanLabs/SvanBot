//! REST calls used around table play: authenticated GETs and buy-in preparation (balance, rebuy).

use super::*;

pub async fn rest_get(http: &reqwest::Client, shared: &Shared, bot: &BotConfig, path: &str) -> Option<Value> {
    let url = format!("{}{}", shared.config.rest_base, path);
    match http.get(&url).bearer_auth(&bot.api_key).send().await {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        Ok(r) => {
            shared.log(&bot.name, "warn", format!("GET {path} -> {}", r.status()));
            None
        }
        Err(e) => {
            shared.log(&bot.name, "warn", format!("GET {path} failed: {e}"));
            None
        }
    }
}

/// How long past a server-scheduled auto-rebuy's due time we wait before rebuying over REST.
pub(super) const AUTO_REBUY_GRACE: Duration = Duration::from_secs(30);

/// The `(chip_balance, chips_at_table)` a funding decision should use: this `/season/me` read when
/// it succeeded, the last payload on record otherwise, both fields from the same payload (0324).
///
/// A failed read is unknown, not a number. Substituting a default here invented a balance (2,000 at
/// a join, 0 at a top-up), and a fabricated 0 also burned the top-up cooldown, so one flaky call
/// played a bot short for ten minutes. `None` says "no balance known": callers wait — or skip the
/// move — rather than decide on a figure nobody measured.
pub(super) fn balance_from(fresh: Option<&Value>, stored: Option<&Value>) -> Option<(i64, i64)> {
    fresh.and_then(funds_of).or_else(|| stored.and_then(funds_of))
}

/// `(balance, chips at the table)` of one `/season/me` payload; `None` when it carries no balance,
/// which reads the same as having no payload at all.
fn funds_of(m: &Value) -> Option<(i64, i64)> {
    Some((m["chip_balance"].as_i64()?, m["chips_at_table"].as_i64().unwrap_or(0)))
}

/// What the next lobby join should do, given the off-table balance.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum BuyInPlan {
    /// Join with this buy-in.
    Join(i64),
    /// Not joinable yet: table chips still returning, or the server's auto-rebuy is pending.
    Wait,
    /// Balance is short and nothing will refill it: rebuy over REST first.
    RestRebuy,
}

/// Plan the buy-in. The server's auto-rebuy (enabled on every join) and `POST /season/rebuy`
/// share one cooldown, so while an `auto_rebuy_scheduled` is outstanding a REST rebuy would only
/// be refused with 429; defer to the server until its due time plus a grace period.
pub(super) fn buy_in_plan(balance: i64, at_table: i64, auto_rebuy_at: Option<Instant>, now: Instant, max_buy_in: i64) -> BuyInPlan {
    if balance >= 1000 {
        return BuyInPlan::Join(balance.min(max_buy_in));
    }
    if at_table > 0 || auto_rebuy_at.is_some_and(|due| now < due + AUTO_REBUY_GRACE) {
        return BuyInPlan::Wait;
    }
    BuyInPlan::RestRebuy
}

/// Decide the buy-in, rebuying first if the off-table balance is below the minimum.
pub(super) async fn prepare_buy_in(http: &reqwest::Client, shared: &Shared, slot: usize, bot: &BotConfig) -> Option<i64> {
    let mut me = rest_get(http, shared, bot, "/season/me").await;
    // Right after leaving a table the server may not have returned the table chips yet; wait for
    // them so a top-up rejoins with the full balance instead of the pre-leave remainder.
    for _ in 0..4 {
        if me.as_ref().and_then(|m| m["chips_at_table"].as_i64()).unwrap_or(0) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        me = rest_get(http, shared, bot, "/season/me").await;
    }
    let stored = shared.bots[slot].read().season.clone();
    if let Some(m) = &me {
        let m = m.clone();
        shared.update(slot, |b| b.season = Some(m));
    }
    let measured = me.as_ref().and_then(funds_of).is_some();
    let Some((balance, at_table)) = balance_from(me.as_ref(), stored.as_ref()) else {
        shared.log(&bot.name, "warn", "join deferred: /season/me unreadable and no balance on record");
        return None;
    };
    if !measured {
        shared.log(&bot.name, "warn", format!("/season/me unusable; planning the join on the last balance on record ({balance})"));
    }
    let auto_rebuy_at = shared.bots[slot].read().auto_rebuy_at;
    match buy_in_plan(balance, at_table, auto_rebuy_at, Instant::now(), shared.config.max_buy_in) {
        BuyInPlan::Join(buy_in) => return Some(buy_in),
        BuyInPlan::Wait => return None,
        BuyInPlan::RestRebuy => {}
    }
    let url = format!("{}/season/rebuy", shared.config.rest_base);
    match http.post(&url).bearer_auth(&bot.api_key).json(&json!({})).send().await {
        Ok(r) if r.status().is_success() => {
            shared.log(&bot.name, "info", "rebuy granted (1,500 chips)");
            shared.update(slot, |b| b.auto_rebuy_at = None);
            Some(1000.max((balance + 1500).min(shared.config.max_buy_in)))
        }
        Ok(r) => {
            let status = r.status();
            let retry = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(String::from);
            let body = r.text().await.unwrap_or_default();
            shared.log(&bot.name, "warn", format!("rebuy refused {status} retry-after={retry:?} {body}"));
            None
        }
        Err(e) => {
            shared.log(&bot.name, "warn", format!("rebuy failed: {e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buy_in_plan_defers_to_a_scheduled_auto_rebuy() {
        let now = Instant::now();
        let due = now + Duration::from_secs(49);
        assert_eq!(buy_in_plan(3000, 0, Some(due), now, 5000), BuyInPlan::Join(3000));
        assert_eq!(buy_in_plan(9000, 0, None, now, 5000), BuyInPlan::Join(5000));
        assert_eq!(buy_in_plan(400, 600, None, now, 5000), BuyInPlan::Wait);
        // The server owns this rebuy's cooldown: a REST rebuy now would only be refused with 429.
        assert_eq!(buy_in_plan(400, 0, Some(due), now, 5000), BuyInPlan::Wait);
        assert_eq!(buy_in_plan(400, 0, Some(due), due + AUTO_REBUY_GRACE / 2, 5000), BuyInPlan::Wait);
        // If the scheduled rebuy never lands, fall back to REST.
        assert_eq!(buy_in_plan(400, 0, Some(due), due + AUTO_REBUY_GRACE, 5000), BuyInPlan::RestRebuy);
        assert_eq!(buy_in_plan(400, 0, None, now, 5000), BuyInPlan::RestRebuy);
    }

    /// 0324: a failed `/season/me` read is unknown, never a number. A joining bot used to be handed
    /// an invented 2,000 (a top-up an invented 0).
    #[test]
    fn a_failed_season_read_is_unknown_not_a_default_balance() {
        let fresh = json!({"chip_balance": 3200, "chips_at_table": 600});
        let older = json!({"chip_balance": 800, "chips_at_table": 0});
        assert_eq!(balance_from(Some(&fresh), Some(&older)), Some((3200, 600)), "this read wins");
        assert_eq!(balance_from(None, Some(&older)), Some((800, 0)), "a failed read falls back to the last payload");
        assert_eq!(balance_from(None, None), None, "nothing on record: no number to decide on");
        assert_eq!(balance_from(Some(&json!({"chips_at_table": 0})), None), None, "a payload without a balance is not a balance");
        assert_eq!(balance_from(Some(&json!(null)), Some(&older)), Some((800, 0)));
        // What the caller does with it: waiting, not joining with 2,000 of invented chips.
        assert_eq!(buy_in_plan(0, 0, None, Instant::now(), 5000), BuyInPlan::RestRebuy);
    }
}
