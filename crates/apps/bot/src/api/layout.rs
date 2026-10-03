//! The operator's board layout (#729): which widgets the dashboard shows, their column and order.
//! One JSON document in the kv store, typed by `web/src/types.ts` (`DashboardLayout`) and reconciled
//! by the board against the widgets the running build has. It is the only server-persisted piece of
//! dashboard customization — panel collapse and compact mode stay in the browser, and the public TV
//! never reads this: it renders its own fixed table view.

use super::*;
use serde::{Deserialize, Serialize};

/// The layout document, exactly as `web/src/types.ts` and `web/src/widgets.tsx` shape it. Widget ids
/// this build does not know are kept as sent: the client drops them, and quietly erasing them here
/// would let an older client lose a newer one's arrangement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct DashboardLayout {
    #[serde(default)]
    left: Vec<String>,
    #[serde(default)]
    center: Vec<String>,
    #[serde(default)]
    right: Vec<String>,
    #[serde(default)]
    hidden: Vec<String>,
}

/// The kv key holding the layout. An empty value means none is stored (the same convention the
/// learner's run key uses), and the browser then keeps the arrangement it remembers.
pub(super) const LAYOUT_KEY: &str = "dashboard.layout.v1";

/// `GET /api/layout`: the saved arrangement, or `null` when none is stored — the client falls back
/// to this browser's copy.
pub(super) async fn get_layout(State(s): State<Arc<Shared>>) -> Result<Json<Value>, ApiError> {
    Ok(Json(stored_json(&s, LAYOUT_KEY)?.unwrap_or(Value::Null)))
}

/// `POST /api/layout`: save the arrangement the Arrange mode just produced; a `null` body stores
/// none, which is how a test returns the sandbox to how it started. Answers with what was stored.
pub(super) async fn save_layout(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let saved = match body {
        Value::Null => Value::Null,
        body => match serde_json::from_value::<DashboardLayout>(body) {
            Ok(layout) => serde_json::to_value(layout).unwrap_or(Value::Null),
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(json!({"detail": format!("layout not understood: {e}")}))).into_response();
            }
        },
    };
    let text = if saved.is_null() { String::new() } else { saved.to_string() };
    if let Err(e) = s.store.put_kv(LAYOUT_KEY, &text) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("could not save the layout: {e}")}))).into_response();
    }
    Json(saved).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_saved_layout_round_trips_and_a_null_clears_it() {
        let shared = Shared::for_test("layout-round-trip", &["A"]);
        let body = json!({"left": ["autonomy"], "center": ["table"], "right": ["monitor"], "hidden": ["updates"]});
        let response = save_layout(State(shared.clone()), Json(body.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let Json(stored) = get_layout(State(shared.clone())).await.unwrap();
        assert_eq!(stored, body);
        save_layout(State(shared.clone()), Json(Value::Null)).await;
        let Json(cleared) = get_layout(State(shared)).await.unwrap();
        assert_eq!(cleared, Value::Null);
    }

    #[tokio::test]
    async fn a_layout_of_the_wrong_shape_is_refused_and_nothing_is_stored() {
        let shared = Shared::for_test("layout-shape", &["A"]);
        let response = save_layout(State(shared.clone()), Json(json!({"left": "autonomy"}))).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let Json(stored) = get_layout(State(shared)).await.unwrap();
        assert_eq!(stored, Value::Null);
    }
}
