//! A hand stored a second time keeps what the first store and its later fills established (#876).
use super::super::*;

fn row(net: Option<i64>, summary: &str) -> HandRow {
    HandRow {
        bot: "A".into(),
        hand_id: "h1".into(),
        table_id: "t".into(),
        ended_at: "2026-10-05T00:00:00Z".into(),
        hero_seat: Some(1),
        hole: "AhKd".into(),
        board: "2c3d4h".into(),
        pot: 100,
        net,
        winners: "A".into(),
        summary: summary.into(),
        showdown: false,
    }
}

fn nets(store: &Store) -> (Option<i64>, Option<f64>) {
    store.read().query_row("SELECT net, ev_net FROM hands WHERE hand_id = 'h1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
}

/// A re-store that does not know the result must not erase the one the row has, and one that
/// changes the result or the record must not leave the all-in EV computed from the old one:
/// `hands_missing_ev` only revisits rows whose `ev_net` is null.
#[test]
fn a_re_store_keeps_a_known_net_and_drops_an_ev_net_it_made_stale() {
    let dir = std::env::temp_dir().join(format!("sv10-store-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("svanbot10.db")).unwrap();
    store.insert_hand(&row(Some(40), "{\"bb\":20}")).unwrap();
    store.set_ev_nets(&[("A".into(), "h1".into(), 12.5)]).unwrap();

    store.insert_hand(&row(None, "{\"bb\":20}")).unwrap();
    assert_eq!(nets(&store), (Some(40), Some(12.5)), "a re-store without a result erased the stored one");

    store.insert_hand(&row(Some(-60), "{\"bb\":20}")).unwrap();
    assert_eq!(nets(&store), (Some(-60), None), "the EV of the old result outlived it");
    let _ = std::fs::remove_dir_all(&dir);
}
