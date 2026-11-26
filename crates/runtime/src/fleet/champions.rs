//! A champion slot for each bot: the parameter set one bot plays by, separate from the fleet's.

use serde_json::{Map, Value, json};
use svanbot_strategy::params::Params;
use svanbot_strategy::style::STANDARD;

use super::{Fleet, params_from, params_json};

const CHAMPIONS: &str = "champions.json";

impl Fleet {
    /// The champion in `bot`'s slot: its own parameter set, or `None` while it has no slot.
    pub fn champion_slot(&self, bot: &str) -> Result<Option<Params>, String> {
        match self.slots()?.get(bot) {
            Some(value) => params_from(value)
                .map(Some)
                .map_err(|error| format!("{CHAMPIONS}: {bot}: {error}")),
            None => Ok(None),
        }
    }

    /// The parameter set `bot` plays by: its own slot's when it has one, the fleet champion's
    /// otherwise.
    pub fn champion_for(&self, bot: &str) -> Result<Params, String> {
        match self.champion_slot(bot)? {
            Some(champion) => Ok(champion),
            None => self.champion(),
        }
    }

    /// Gives `bot` a new slot, seeded from the fleet champion with the pins of the style it is
    /// assigned applied. A slot that exists is never replaced: a bot's learned champion is not a
    /// seed's to overwrite.
    pub fn seed_champion(&self, bot: &str) -> Result<Params, String> {
        let _lock = self.lock_fleet_file(CHAMPIONS)?;
        let mut bots = self.slots()?;
        if bots.contains_key(bot) {
            return Err(format!("{CHAMPIONS}: {bot} already has a champion slot"));
        }
        let assigned = self
            .assignment(bot)?
            .map_or(&STANDARD, |assignment| assignment.style);
        let seeded = assigned.seed(self.champion()?);
        bots.insert(bot.into(), params_json(&seeded));
        self.write(CHAMPIONS, &json!({ "bots": bots }).to_string())?;
        Ok(seeded)
    }

    /// Puts `champion` in `bot`'s slot, creating the slot if it had none. Only `bot`'s entry
    /// changes: every other bot's slot is left as it was.
    pub fn set_champion(&self, bot: &str, champion: &Params) -> Result<(), String> {
        let _lock = self.lock_fleet_file(CHAMPIONS)?;
        let mut bots = self.slots()?;
        bots.insert(bot.into(), params_json(champion));
        self.write(CHAMPIONS, &json!({ "bots": bots }).to_string())
    }

    fn slots(&self) -> Result<Map<String, Value>, String> {
        self.bots_in(CHAMPIONS)
    }
}
