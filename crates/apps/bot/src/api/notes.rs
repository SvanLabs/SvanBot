//! The operator's notes scratchpad (#730): one plain-text document, saved while it is typed. One
//! JSON value in the kv store (`dashboard.notes.v1`), typed by `web/src/types.ts` (`DashboardNotes`);
//! an empty text clears the key (the store's convention for "nothing stored"). The public TV does
//! not mount it, and never renders the board the panel sits on.

use super::*;
use serde::{Deserialize, Serialize};

/// The notes document, exactly as `web/src/types.ts` shapes it: one scratchpad, no list, no folders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct DashboardNotes {
    text: String,
}

/// The panel is a few lines of operator memory, not a document store; a note past this is refused
/// rather than kept as an unbounded row in the shared store. The client's counter caps at the same
/// number (`web/src/notes.tsx`).
pub(super) const NOTES_MAX_CHARS: usize = 20_000;

/// The kv key holding the note. An empty value means none is stored (the same convention the layout
/// and the learner's run key use), and the browser then keeps the copy it remembers.
pub(super) const NOTES_KEY: &str = "dashboard.notes.v1";

/// `GET /api/notes`: the saved scratchpad, or `null` when none is stored — the client then shows the
/// copy this browser remembers, and offers it back.
pub(super) async fn get_notes(State(s): State<Arc<Shared>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(stored_json(&s, NOTES_KEY)?.unwrap_or(Value::Null)))
}

/// `POST /api/notes`: save the scratchpad. A body that is not `{text: string}`, or text past the cap,
/// is refused with 400 and changes nothing; empty text clears the stored note.
pub(super) async fn save_notes(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let notes = match serde_json::from_value::<DashboardNotes>(body) {
        Ok(notes) if notes.text.chars().count() <= NOTES_MAX_CHARS => notes,
        Ok(_) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"detail": format!("a note is limited to {NOTES_MAX_CHARS} characters")})))
                .into_response();
        }
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"detail": format!("notes not understood: {e}")}))).into_response();
        }
    };
    let saved = serde_json::to_value(&notes).unwrap_or(Value::Null);
    let text = if notes.text.is_empty() { String::new() } else { saved.to_string() };
    if let Err(e) = s.store.put_kv(NOTES_KEY, &text) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("could not save the notes: {e}")}))).into_response();
    }
    Json(saved).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_saved_note_round_trips_and_empty_text_clears_it() {
        let shared = Shared::for_test("notes-round-trip", &["A"]);
        let body = json!({"text": "watch the turn c-bets against the limpers"});
        let response = save_notes(State(shared.clone()), Json(body.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let Json(stored) = get_notes(State(shared.clone())).await.unwrap();
        assert_eq!(stored, body);
        save_notes(State(shared.clone()), Json(json!({"text": ""}))).await;
        let Json(cleared) = get_notes(State(shared)).await.unwrap();
        assert_eq!(cleared, Value::Null);
    }

    #[tokio::test]
    async fn notes_of_the_wrong_shape_or_over_the_cap_are_refused_and_nothing_is_stored() {
        let shared = Shared::for_test("notes-shape", &["A"]);
        let too_long = json!({"text": "x".repeat(NOTES_MAX_CHARS + 1)});
        for body in [json!({"text": 7}), json!("a string"), json!({}), too_long] {
            assert_eq!(save_notes(State(shared.clone()), Json(body)).await.status(), StatusCode::BAD_REQUEST);
        }
        let Json(stored) = get_notes(State(shared)).await.unwrap();
        assert_eq!(stored, Value::Null);
    }
}
