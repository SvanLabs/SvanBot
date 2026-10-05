//! svanbot10 poker core: one path (`sv10_core::<module>`) over the layered crates
//! `sv10-cards` → `sv10-equity` / `sv10-engine` / `sv10-nn` → `sv10-model` → `sv10-policy`.

pub mod inputs;

pub use sv10_cards::cards;
pub use sv10_cards::eval;
pub use sv10_cards::range;
pub use sv10_engine::engine;
pub use sv10_engine::situation;
pub use sv10_equity::abandon;
pub use sv10_equity::equity;
pub use sv10_equity::preflop;
pub use sv10_equity::tables;
pub use sv10_model::adapt;
pub use sv10_model::allin;
pub use sv10_model::calibrate;
pub use sv10_model::features;
pub use sv10_model::flow;
pub use sv10_model::history;
pub use sv10_model::model;
pub use sv10_model::oprange;
pub use sv10_model::phh;
pub use sv10_model::residual;
pub use sv10_model::sizetell;
pub use sv10_nn::nn;
pub use sv10_policy::agents;
pub use sv10_policy::bench;
pub use sv10_policy::hardware;
pub use sv10_policy::policy;
pub use sv10_policy::sim;
