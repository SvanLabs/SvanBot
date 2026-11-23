//! Messages to the venue.

use serde_json::json;
use svanbot_core::rules::Chips;

/// An action as the venue names it. A raise names the street total it rises to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VenueAction {
    Fold,
    Check,
    Call,
    Raise(Chips),
    AllIn,
}

/// One message to the venue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outbound {
    JoinLobby {
        buy_in: Chips,
    },
    SetAutoRebuy {
        enabled: bool,
    },
    /// Echoes the turn's hand and token. The client action id makes a retry safe.
    Action {
        hand_id: String,
        action: VenueAction,
        client_action_id: String,
        turn_token: String,
    },
    LeaveTable,
    ResyncRequest {
        table_id: String,
        last_table_seq: u64,
    },
}

impl Outbound {
    /// The message as the venue reads it.
    pub fn to_json(&self) -> String {
        let message = match self {
            Self::JoinLobby { buy_in } => json!({"type": "join_lobby", "buy_in": buy_in}),
            Self::SetAutoRebuy { enabled } => json!({"type": "set_auto_rebuy", "enabled": enabled}),
            Self::Action {
                hand_id,
                action,
                client_action_id,
                turn_token,
            } => {
                let (name, amount) = match action {
                    VenueAction::Fold => ("fold", None),
                    VenueAction::Check => ("check", None),
                    VenueAction::Call => ("call", None),
                    VenueAction::Raise(to) => ("raise", Some(to)),
                    VenueAction::AllIn => ("all_in", None),
                };
                let mut message = json!({
                    "type": "action",
                    "hand_id": hand_id,
                    "action": name,
                    "client_action_id": client_action_id,
                    "turn_token": turn_token,
                });
                if let Some(to) = amount {
                    message["amount"] = json!(to);
                }
                message
            }
            Self::LeaveTable => json!({"type": "leave_table"}),
            Self::ResyncRequest {
                table_id,
                last_table_seq,
            } => {
                json!({"type": "resync_request", "table_id": table_id, "last_table_seq": last_table_seq})
            }
        };
        message.to_string()
    }
}
