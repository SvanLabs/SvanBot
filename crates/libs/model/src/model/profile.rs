//! Turning one player's raw tallies into the [`Profile`] decisions read: every rate shrunk toward
//! the population aggregate, so a handful of samples can't swing a decision.

use super::{Counter, ModelStore, PlayerStats, Profile, W_POST, W_PRE, defaults};

impl ModelStore {
    fn pop_rate(&self, c: &Counter, default: f32) -> f32 {
        // The pool aggregate replaces hand-tuned defaults once it is large.
        c.rate(default, 150.0)
    }

    /// Shrunk rates for `name` (an unknown name gets the population rates).
    pub fn profile(&self, name: &str) -> Profile {
        let empty = PlayerStats::default();
        self.profile_of(self.players.get(name).unwrap_or(&empty), name)
    }

    /// [`Profile`] from one player's tallies, shrunk toward the population rates; `name` supplies the
    /// per-player fits the learner installed (0210/0214/0223/0234).
    pub(crate) fn profile_of(&self, s: &PlayerStats, name: &str) -> Profile {
        let p = &self.population;
        let pr = |own: &Counter, pop: &Counter, d: f32, w: f32| own.rate(self.pop_rate(pop, d), w);
        let arr = |own: &[Counter; 3], pop: &[Counter; 3], d: [f32; 3], w: f32| {
            [pr(&own[0], &pop[0], d[0], w), pr(&own[1], &pop[1], d[1], w), pr(&own[2], &pop[2], d[2], w)]
        };
        use defaults as d;
        Profile {
            hands: s.hands,
            vpip: pr(&s.vpip, &p.vpip, d::VPIP, W_PRE),
            pfr: pr(&s.pfr, &p.pfr, d::PFR, W_PRE),
            open_raise: pr(&s.open_raise, &p.open_raise, d::OPEN_RAISE, W_PRE),
            limp: pr(&s.limp, &p.limp, d::LIMP, W_PRE),
            three_bet: pr(&s.three_bet, &p.three_bet, d::THREE_BET, W_PRE),
            call_open: pr(&s.call_open, &p.call_open, d::CALL_OPEN, W_PRE),
            fold_to_3bet: pr(&s.fold_to_3bet, &p.fold_to_3bet, d::FOLD_TO_3BET, 6.0),
            four_bet: pr(&s.four_bet, &p.four_bet, d::FOUR_BET, 6.0),
            fold_to_4bet: pr(&s.fold_to_4bet, &p.fold_to_4bet, d::FOLD_TO_4BET, 5.0),
            bet_first: arr(&s.bet_first, &p.bet_first, d::BET_FIRST, W_POST),
            fold_vs_bet: arr(&s.fold_vs_bet, &p.fold_vs_bet, d::FOLD_VS_BET, W_POST),
            raise_vs_bet: arr(&s.raise_vs_bet, &p.raise_vs_bet, d::RAISE_VS_BET, W_POST),
            fold_vs_size: arr(&s.fold_vs_size, &p.fold_vs_size, d::FOLD_VS_SIZE, W_POST),
            cbet: pr(&s.cbet, &p.cbet, d::CBET, 8.0),
            fold_to_cbet: pr(&s.fold_to_cbet, &p.fold_to_cbet, d::FOLD_TO_CBET, 8.0),
            wtsd: pr(&s.wtsd, &p.wtsd, d::WTSD, W_POST),
            river_bluff: pr(&s.river_bluff, &p.river_bluff, d::RIVER_BLUFF, 5.0),
            open_pos: {
                let overall = pr(&s.open_raise, &p.open_raise, d::OPEN_RAISE, W_PRE);
                let scale = [0.7f32, 1.45, 1.0];
                [0, 1, 2].map(|g| s.open_pos[g].rate((overall * scale[g]).clamp(0.01, 0.95), 15.0))
            },
            vpip_pos: {
                let overall = pr(&s.vpip, &p.vpip, d::VPIP, W_PRE);
                let scale = [0.8f32, 1.3, 1.05];
                [0, 1, 2].map(|g| s.vpip_pos[g].rate((overall * scale[g]).clamp(0.02, 0.98), 15.0))
            },
            won_showdown: pr(&s.won_showdown, &p.won_showdown, 0.5, 8.0),
            confidence: s.hands / (s.hands + 40.0),
            response_ratio: self.response_ratios.get(name).copied().unwrap_or(crate::residual::UNIT_RATIO),
            fold_logit_offset: self.fold_offsets.get(name).copied().unwrap_or(0.0),
            size_tell: self.size_tells.get(name).copied().unwrap_or(0.0),
            think_base: [s.think[0].typical_ms(), s.think[1].typical_ms()],
        }
    }
}
