//! Print the policy's candidate EVs for a few canonical spots.

use sv10_core::cards::{Card, parse_cards};
use sv10_core::engine::{Action, Hand};
use sv10_core::model::ModelStore;
use sv10_core::policy::{Params, decide};
use sv10_core::situation::Situation;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

fn spot(title: &str, holes: &[[&str; 2]], board: [&str; 5], button: usize, actions: &[Action], hero: usize) {
    let seats = holes.iter().map(|h| Hand::seat_state(2000, [Card::parse(h[0]).unwrap(), Card::parse(h[1]).unwrap()])).collect();
    let runout: [Card; 5] = parse_cards(&board).unwrap().try_into().unwrap();
    let mut hand = Hand::with_cards(seats, runout, button, 10, 20);
    for &a in actions {
        hand.apply(a).unwrap();
    }
    assert_eq!(hand.actor(), Some(hero), "{title}: actor mismatch");
    let names: Vec<String> = (0..holes.len()).map(|i| format!("p{i}")).collect();
    let sit = Situation::from_hand(&hand, hero, &names);
    let mut rng = SmallRng::seed_from_u64(1);
    let d = decide(&sit, &ModelStore::default(), &Params::default(), &mut rng);
    println!(
        "== {title}: hero {}{} board {:?} pot {} call {} -> {} {:?} (eq {:.2})",
        sit.hole[0],
        sit.hole[1],
        sit.board.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        sit.pot,
        sit.call_amount,
        d.action_name,
        d.amount,
        d.equity
    );
    for c in &d.candidates {
        println!("   {:>6} {:>6?} ev {:>8.1} fold {:.2} eqc {:.2}", c.action, c.amount, c.ev, c.fold_prob, c.equity_called);
    }
}

/// Resident memory of this process from `/proc/self/status` and `/proc/self/smaps_rollup` (MB):
/// anonymous (private heap), file-backed (mapped files; shared between processes when the same file
/// is mapped read-only), and the proportional set size that splits shared pages between processes.
fn memory_mb() -> String {
    let field = |text: &str, key: &str| {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<f64>().ok())
            .map(|kb| kb / 1024.0)
    };
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let show = |v: Option<f64>| v.map(|m| format!("{m:.0}")).unwrap_or_else(|| "?".into());
    format!(
        "RSS {} MB (anon {}, file {}), PSS {} MB, peak {} MB",
        show(field(&status, "VmRSS:")),
        show(field(&status, "RssAnon:")),
        show(field(&status, "RssFile:")),
        show(field(&rollup, "Pss:")),
        show(field(&status, "VmHWM:"))
    )
}

fn bench() {
    use sv10_core::eval::eval;
    println!("memory at start: {}", memory_mb());
    for (len, name) in [(3, "flop"), (4, "turn")] {
        let t = std::time::Instant::now();
        let loaded = sv10_core::tables::loaded(len).is_some();
        println!("{name} strength table: {} in {:.0} ms", if loaded { "loaded" } else { "missing" }, t.elapsed().as_secs_f64() * 1000.0);
    }
    println!("memory with both tables: {}", memory_mb());
    let t = std::time::Instant::now();
    let _ = sv10_core::preflop::table();
    println!("preflop class table (cold): {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = std::time::Instant::now();
    let _ = sv10_core::preflop::top_range_equity();
    println!("top-range equity ladder (cold): {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = std::time::Instant::now();
    let _ = sv10_core::hardware::detect();
    println!("hardware detect: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let t = std::time::Instant::now();
    let mut x = 0u64;
    let mut acc = 0u64;
    for i in 0..20_000_000u64 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut m = 0u64;
        let mut k = x;
        while m.count_ones() < 7 {
            m |= 1 << (k % 52);
            k = k.rotate_left(7).wrapping_add(i);
        }
        acc = acc.wrapping_add(eval(m) as u64);
    }
    println!("eval: {:.1} ns/hand (checksum {acc})", t.elapsed().as_nanos() as f64 / 20_000_000.0);
    let holes = [["Ac", "Kd"], ["Qc", "8d"], ["7h", "7s"]];
    let seats = holes.iter().map(|h| Hand::seat_state(2000, [Card::parse(h[0]).unwrap(), Card::parse(h[1]).unwrap()])).collect();
    let runout: [Card; 5] = parse_cards(&["Kh", "7s", "2d", "9h", "4c"]).unwrap().try_into().unwrap();
    let mut hand = Hand::with_cards(seats, runout, 0, 10, 20);
    for a in [Action::RaiseTo(50), Action::Call, Action::Call, Action::Check, Action::Check] {
        hand.apply(a).unwrap();
    }
    let names: Vec<String> = (0..3).map(|i| format!("p{i}")).collect();
    let actor = hand.actor().unwrap();
    let sit = Situation::from_hand(&hand, actor, &names);
    let models = ModelStore::default();
    let params = Params::default();
    let mut rng = SmallRng::seed_from_u64(1);
    let _ = decide(&sit, &models, &params, &mut rng);
    let t = std::time::Instant::now();
    for _ in 0..200 {
        let _ = decide(&sit, &models, &params, &mut rng);
    }
    println!("decide (3-way flop, warm cache): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
    for (samples, chunks) in [(10_000, 8), (40_000, 8), (400_000, 8), (1_000_000, 8)] {
        let live = Params { samples, deal_chunks: chunks, ..Params::default() };
        let t = std::time::Instant::now();
        let runs = (400_000 / samples).clamp(3, 50);
        for _ in 0..runs {
            let _ = decide(&sit, &models, &live, &mut rng);
        }
        println!("decide {samples} samples in {chunks} parallel chunks: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / runs as f64);
    }
    let t = std::time::Instant::now();
    for _ in 0..200 {
        let _ = sv10_core::oprange::estimate_ranges(&sit, &models, &Default::default());
    }
    println!("  estimate_ranges: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
    let ranges = sv10_core::oprange::estimate_ranges(&sit, &models, &Default::default());
    let refs: Vec<&sv10_core::range::Range> = ranges.values().collect();
    let t = std::time::Instant::now();
    for _ in 0..200 {
        let _ = sv10_core::equity::equity_vs_ranges(sit.hole, &sit.board, &refs, 2500, &mut rng);
    }
    println!("  equity 2500 samples x2 opps: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
    let t = std::time::Instant::now();
    for _ in 0..200 {
        let _ = sv10_core::oprange::perceived_range(&sit, models.profile("x"), &Default::default());
    }
    println!("  perceived_range: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
    let st = sv10_core::oprange::board_strengths(&sit.board);
    let t = std::time::Instant::now();
    for _ in 0..200 {
        let _ = sv10_core::oprange::global_pct(&st);
    }
    println!("  global_pct: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
    let net = sv10_core::nn::Mlp::new(&[sv10_core::features::N_FEATURES, 48, 24, 3], 11);
    let x: Vec<f32> = (0..sv10_core::features::N_FEATURES).map(|i| (i as f32 * 0.37).sin()).collect();
    let mask = [true; 3];
    let t = std::time::Instant::now();
    let mut acc = 0f32;
    for _ in 0..100_000 {
        acc += net.predict(&x, &mask)[0];
    }
    println!(
        "  mlp predict ({}-48-24-3): {:.1} us (checksum {acc:.3})",
        sv10_core::features::N_FEATURES,
        t.elapsed().as_secs_f64() * 1e6 / 100_000.0
    );
    let t = std::time::Instant::now();
    let mut r2 = SmallRng::seed_from_u64(5);
    let _ = sv10_core::equity::combo_strengths(&sit.board, 80, &mut r2);
    println!("  combo_strengths flop (cold): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
    println!("memory at the end: {}", memory_mb());
}

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        return bench();
    }
    if std::env::args().any(|a| a == "--hardware") {
        println!("{}", serde_json::to_string_pretty(&sv10_core::hardware::detect()).unwrap());
        return;
    }
    let six = |h: [&'static str; 2]| vec![["2c", "3d"], ["4c", "5d"], ["6h", "8d"], h, ["9c", "Td"], ["Jc", "Qd"]];
    let b = ["Kh", "7s", "2d", "9h", "4c"];
    for h in [["7h", "2s"], ["Ah", "Ks"], ["9s", "8s"], ["Qd", "Jh"], ["5s", "5h"]] {
        spot(&format!("UTG open {}{}", h[0], h[1]), &six(h), b, 0, &[], 3);
    }
    // 0277: the *same* hand opened from the button — hero last to act preflop, only the two blinds
    // still to come. Card for card and pot for pot this is the spot above with every seat between
    // folded, so any difference in the numbers is the position and nothing else. It is the cheapest
    // way to see whether the policy prices position at all, and it has an answer either way, which
    // is the point of printing both.
    let btn = |h: [&'static str; 2]| vec![["2c", "3d"], ["4c", "5d"], ["6h", "8d"], ["9c", "Td"], ["Jc", "Qd"], [h[0], h[1]]];
    for h in [["7h", "2s"], ["Ah", "Ks"], ["9s", "8s"], ["Qd", "Jh"], ["5s", "5h"]] {
        spot(&format!("BTN open {}{}", h[0], h[1]), &btn(h), b, 5, &[Action::Fold, Action::Fold, Action::Fold], 5);
    }
    // Hero on the button (seat 0) facing a UTG (seat 3) open.
    let holes = vec![["Qh", "Jh"], ["4c", "5d"], ["6h", "8d"], ["As", "Td"], ["9c", "2d"], ["Jc", "3d"]];
    spot("BTN vs UTG open QJs", &holes, b, 0, &[Action::RaiseTo(50), Action::Fold, Action::Fold], 0);
    let holes = vec![["7h", "6c"], ["4c", "5d"], ["6h", "8d"], ["As", "Td"], ["9c", "2d"], ["Jc", "3d"]];
    spot("BTN vs UTG open 76o", &holes, b, 0, &[Action::RaiseTo(50), Action::Fold, Action::Fold], 0);
    // Heads-up flop: BTN opened, BB called, BB checks, BTN to act.
    let hu = |h: [&'static str; 2]| vec![h, ["Qc", "8d"]];
    for h in [["Ac", "Kd"], ["6c", "5d"], ["7c", "7d"], ["Ah", "Qh"]] {
        spot(
            &format!("HU flop cbet {}{}", h[0], h[1]),
            &hu(h),
            ["Kh", "7s", "2d", "9h", "4c"],
            0,
            &[Action::RaiseTo(50), Action::Call, Action::Check],
            0,
        );
    }
    // Facing a flop bet heads-up.
    for h in [["Ac", "Kd"], ["6c", "5d"], ["Qs", "Qd"]] {
        spot(
            &format!("HU facing flop bet {}{}", h[0], h[1]),
            &hu(h),
            ["Kh", "7s", "2d", "9h", "4c"],
            0,
            &[Action::RaiseTo(50), Action::Call, Action::RaiseTo(70)],
            0,
        );
    }
}
