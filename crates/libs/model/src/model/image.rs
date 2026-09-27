//! The image opponents form of *our* play (0321). `observe` skips our own seat — we are not our own
//! opponent — so our play was never tallied anywhere, while the pricing priced every responder
//! against the range they read us for and so read us as an unknown player. This module keeps that
//! image: our own seat's tallies per bot and in aggregate, and the profile the pricing reads.

use super::{HandSummary, ModelStore, PlayerStats, Profile, hand_stats};

/// The fleet's aggregate image of our own play, under the reserved name the pricing asks for when
/// it reads how opponents see us ([`ModelStore::hero_seen_view`]).
pub const HERO_SEEN_ALL: &str = "\u{0}hero-as-seen";

/// Prefix of one bot's own image of our play: `HERO_SEEN_ONE + <bot name>`. Unknown names fall
/// back to [`HERO_SEEN_ALL`], so a bot seen for the first time still gets the fleet's image.
pub const HERO_SEEN_ONE: &str = "\u{0}hero-as-seen:";

impl ModelStore {
    /// Observe one of *our own* hands: every other seat's tallies, and the image opponents form of
    /// our play. Both always travel together, so every path that folds a hand of ours — live, tailed
    /// or rebuilt — has this one entry point and no path can feed one half alone. A treatment-arm
    /// hand reaches neither half (0291).
    pub fn observe_own_hand(&mut self, hand: &HandSummary, bot: &str, w: f32) {
        self.observe_weighted(hand, Some(bot), w);
        self.observe_image_weighted(hand, bot, w);
    }

    /// Add our play in `hand` to the image opponents form of `hero`, under the hero's name and in
    /// the fleet aggregate, with the same recency half-life as any player's tallies.
    pub fn observe_image(&mut self, hand: &HandSummary, hero: &str) {
        self.observe_image_weighted(hand, hero, 1.0);
    }

    /// As [`observe_image`](ModelStore::observe_image), with every count scaled by `w`.
    pub fn observe_image_weighted(&mut self, hand: &HandSummary, hero: &str, w: f32) {
        let Some(seat) = hand.players.iter().find(|(_, n)| n == hero).map(|(s, _)| *s) else { return };
        let Some(own) = hand_stats(hand).remove(&seat) else { return };
        let half_life = self.half_life_hands;
        for key in [format!("{HERO_SEEN_ONE}{hero}"), HERO_SEEN_ALL.to_string()] {
            let image = self.hero_seen.entry(key).or_default();
            if half_life > 0.0 {
                image.decay_counters(0.5f32.powf(1.0 / half_life));
            }
            image.merge_weighted(&own, w);
        }
    }

    /// The image pricing assumes opponents hold of our player `name`, with its sample weight scaled
    /// by `weight` (0 = none, 1 = all of it). `None` when nothing has been observed yet, so callers
    /// keep the population view; a thin image is shrunk toward the population by
    /// [`profile_of`](ModelStore::profile_of) like any other player's.
    pub fn hero_seen_profile(&self, name: &str, weight: f64) -> Option<Profile> {
        if weight.is_nan() || weight <= 0.0 {
            return None;
        }
        let one = format!("{HERO_SEEN_ONE}{name}");
        let s = self.hero_seen.get(&one).or_else(|| self.hero_seen.get(HERO_SEEN_ALL))?;
        let mut scaled = PlayerStats::default();
        scaled.merge_weighted(s, weight as f32);
        Some(self.profile_of(&scaled, name))
    }

    /// The profile to price opponents' responses against from our seat: the image of our own play
    /// when `hero_image` turns it on and there is one, the view of an unobserved player (the
    /// population rates) otherwise — which is exactly what the pricing assumed before 0321.
    pub fn hero_seen_view(&self, name: &str, hero_image: f64) -> Profile {
        self.hero_seen_profile(name, hero_image).unwrap_or_else(|| self.profile(HERO_SEEN_ALL))
    }
}
