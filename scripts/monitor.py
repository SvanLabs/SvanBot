#!/usr/bin/env python3
"""Smart results monitor: wins/losses per bot and per opponent, read-only against the live store.

Prints one line per event worth acting on (stdout is an event stream):
  SUMMARY  every --summary-min minutes: fleet and per-bot net, top donors and takers in the window
  BIGLOSS  a single hand losing at least --big-loss-bb big blinds, with who won it
  NEMESIS  an opponent newly beating us: the chips that moved between us and them in champion
           hands, at 95% family-wise across every opponent with >= --min-hands shared hands (0221).
           Experiment-mode treatment hands are left out, as in the bot's own ledger (0361)
  STALL    a bot with no completed hand for --stall-min minutes
  ERROR    new error-level events from the fleet log
Usage: scripts/monitor.py [--once] [--interval 300]
"""
import argparse, json, math, re, sqlite3, sys, time
from collections import defaultdict
from statistics import NormalDist
from datetime import datetime, timezone
from pathlib import Path

DB = Path(__file__).resolve().parent.parent / "artifacts" / "svanbot10.db"


def connect():
    return sqlite3.connect(f"file:{DB}?mode=ro", uri=True, timeout=10)


def now():
    return datetime.now().astimezone().strftime("%H:%M")  # local time, like the operator's clock


class Tally:
    __slots__ = ("n", "s", "sq", "low")

    def __init__(self):
        self.n, self.s, self.sq, self.low = 0, 0.0, 0.0, math.inf

    def add(self, x):
        self.n += 1
        self.s += x
        self.sq += x * x
        self.low = min(self.low, x)

    def mean(self):
        return self.s / self.n if self.n else 0.0

    def mean_without_biggest_loss(self):
        """Mean with the single worst hand removed, so one cooler does not read as a leak (0209)."""
        if self.n < 2:
            return self.mean()
        return (self.s - self.low) / (self.n - 1)

    def upper(self, z=1.96):
        if self.n < 2:
            return math.inf
        m = self.mean()
        var = max(self.sq / self.n - m * m, 0.0)
        return m + z * math.sqrt(var / self.n)

    def upper95(self):
        return self.upper(1.96)


def family_z(tested):
    """Standard errors for a 2.5% chance of any false verdict among `tested` opponents (Bonferroni;
    mirrors `headtohead::family_z`)."""
    return NormalDist().inv_cdf(1 - 0.025 / max(tested, 1))


def contributions(s, pot):
    """Chips each dealt-in seat put in, uncalled chips returned; None unless it reproduces `pot`
    (mirrors `sv10_core::flow::contributions`)."""
    seats = sorted(p[0] for p in s.get("players", []))
    if len(seats) < 2:
        return None
    stacks = {a: b for a, b in s.get("stacks", [])}
    stack = lambda seat: stacks.get(seat, math.inf)
    b, bb = s["button"], s["bb"]
    after = [x for x in seats if x > b] + [x for x in seats if x <= b]
    sb, big = (b, after[0]) if len(seats) == 2 else (after[0], after[1])
    out = {x: 0 for x in seats}
    out[sb] += min(bb // 2, stack(sb))
    out[big] += min(bb, stack(big))
    hist = s.get("history", [])
    if hist and hist[0]["street"] == "Preflop" and hist[0]["pot_before"] != sum(out.values()):
        return None
    for i, r in enumerate(hist):
        if i + 1 < len(hist):
            chips = hist[i + 1]["pot_before"] - r["pot_before"]
        elif r["kind"] in ("Fold", "Check"):
            chips = 0
        elif r["kind"] == "AllIn":
            if r["seat"] not in stacks:
                return None
            chips = stacks[r["seat"]] - out.get(r["seat"], 0)
        else:
            chips = r["to"] - r["bet_before"]
        if chips < 0 or r["seat"] not in out:
            return None
        out[r["seat"]] += chips
    ranked = sorted(out.values(), reverse=True)
    if ranked[0] > ranked[1]:
        out[max(out, key=out.get)] = ranked[1]
    return out if sum(out.values()) == pot else None


def showdown_rank(board, hole):
    """Comparable seven-card rank, or None when the recorded cards are incomplete or invalid."""
    from itertools import combinations
    cards = list(board) + list(hole)
    if len(board) != 5 or len(hole) != 2 or len(set(cards)) != 7:
        return None
    if any(len(card) != 2 or card[0] not in '23456789TJQKA' or card[1] not in 'cdhs' for card in cards):
        return None
    parsed = [('23456789TJQKA'.index(card[0]) + 2, card[1]) for card in cards]

    def five_rank(five):
        values = sorted((value for value, _ in five), reverse=True)
        groups = sorted(((values.count(value), value) for value in set(values)), reverse=True)
        flush = len({suit for _, suit in five}) == 1
        straight = (5 if values == [14, 5, 4, 3, 2] else values[0]) if (
            len(set(values)) == 5 and (values[0] - values[-1] == 4 or values == [14, 5, 4, 3, 2])) else None
        if flush and straight:
            return (8, straight)
        if groups[0][0] == 4:
            return (7, groups[0][1], groups[1][1])
        if [count for count, _ in groups] == [3, 2]:
            return (6, groups[0][1], groups[1][1])
        if flush:
            return (5, *values)
        if straight:
            return (4, straight)
        if groups[0][0] == 3:
            return (3, groups[0][1], *sorted((value for count, value in groups if count == 1), reverse=True))
        pairs = sorted((value for count, value in groups if count == 2), reverse=True)
        if len(pairs) == 2:
            return (2, *pairs, next(value for count, value in groups if count == 1))
        if pairs:
            return (1, *pairs, *sorted((value for count, value in groups if count == 1), reverse=True))
        return (0, *values)

    return max(five_rank(five) for five in combinations(parsed, 5))


def flow_to_hero(s, pot, winners, hero):
    """Per-pot opponent transfers; None when contribution or side-pot evidence is insufficient."""
    c = contributions(s, pot)
    if c is None or hero not in c:
        return None
    won = [seat for seat, name in s['players'] if name in winners]
    folded = {record['seat'] for record in s.get('history', []) if record['kind'] == 'Fold'}
    if not won or folded.intersection(won):
        return None
    side_pots = len({c[seat] for seat in won}) > 1
    shown = dict(s.get('shown', []))
    out = {seat: 0.0 for seat, _ in s['players'] if seat != hero}
    paid = set()
    previous = 0
    for level in sorted({amount for amount in c.values() if amount > 0}):
        contributors = {seat for seat in c if c[seat] >= level}
        eligible = [seat for seat in won if c[seat] >= level]
        if not eligible:
            return None
        if side_pots and len(eligible) > 1:
            ranks = {seat: showdown_rank(s.get('board', []), shown.get(seat, [])) for seat in eligible}
            if any(rank is None for rank in ranks.values()):
                return None
            best = max(ranks.values())
            eligible = [seat for seat in eligible if ranks[seat] == best]
        paid.update(eligible)
        chips = (level - previous) / len(eligible)
        if hero in contributors:
            for seat in contributors - {hero}:
                if hero in eligible and seat not in eligible:
                    out[seat] += chips
                elif seat in eligible and hero not in eligible:
                    out[seat] -= chips
        previous = level
    return out if set(won).issubset(paid) else None

def toughest(ledger, min_hands, k=3):
    """The `k` opponents we do worst against once each one's biggest pot is set aside (0209)."""
    return sorted((t.mean_without_biggest_loss(), p, t) for p, t in ledger.items() if t.n >= min_hands)[:k]


def format_toughest(worst):
    out = []
    for trimmed, p, t in worst:
        extra = "" if abs(trimmed - t.mean()) < 0.5 else f", {trimmed * 100:+.0f} without its biggest pot"
        out.append(f"{p} {t.mean() * 100:+.0f}bb/100{extra} ({t.n})")
    return ", ".join(out)


def fleet_names(conn):
    return {r[0] for r in conn.execute("SELECT DISTINCT bot FROM hands")}


def active_bots(conn, hours, at=None):
    """Bots with a completed hand in the last `hours`: the stall watch skips retired names (0184)."""
    at = time.time() if at is None else at
    out = set()
    for bot, ended in conn.execute("SELECT bot, MAX(ended_at) FROM hands GROUP BY bot"):
        # SQLite stores nanoseconds; fromisoformat takes at most six fraction digits.
        ended = re.sub(r"(\.\d{6})\d+", r"\1", ended or "")
        try:
            if at - datetime.fromisoformat(ended).timestamp() <= hours * 3600:
                out.add(bot)
        except ValueError:
            continue
    return out


def read_hands(conn, after):
    """Every hand after `after`, with a flag for a treatment-arm hand (0291). The flag is the SQL of
    `sv10_store::store::provenance::ordinary_hand`, negated: the ledger leaves those hands out (0361)."""
    rows = conn.execute(
        "SELECT rowid, bot, hand_id, ended_at, net, summary, winners, COALESCE(pot, 0), "
        "EXISTS (SELECT 1 FROM hand_provenance p WHERE p.bot = hands.bot AND p.hand_id = hands.hand_id AND p.arm = 'treatment') "
        "FROM hands WHERE rowid > ? ORDER BY rowid",
        (after,),
    )
    for rowid, bot, hand_id, ended, net, summary, winners, pot, treat in rows:
        try:
            s = json.loads(summary) if summary else {}
        except ValueError:
            s = {}
        bb = s.get("bb") or 20
        players = [p[1] for p in s.get("players", [])]
        yield rowid, bot, hand_id, ended, net, bb, players, winners or "", s, pot, treat


def add_flows(ledger, fleet, bot, s, pot, winners, window=None, treat=False):
    """Add one hand's chip flows (in bb) to the all-time ledger, and to `window` ({name: bb}) when
    given; False when it does not reconcile.

    A treatment hand is left out of the ledger — its verdict moves the fleet between tables, and a
    challenger's hands are not evidence about the policy that plays there (0361) — and stays in
    `window`, which is a results read where the chips are the answer."""
    hero = next((x for x, n in s.get("players", []) if n == bot), None)
    flows = flow_to_hero(s, pot, [w for w in winners.split(",") if w], hero) if hero is not None else None
    if flows is None:
        return False
    names = dict((x, n) for x, n in s["players"])
    for x, chips in flows.items():
        if names[x] not in fleet:
            if not treat:
                ledger[names[x]].add(chips / s["bb"])
            if window is not None:
                window[names[x]] += chips / s["bb"]
    return True


def nemeses_of(ledger, min_hands):
    """Opponents beating us at 95% family-wise across all with `min_hands` hands (0221)."""
    tested = [p for p, t in ledger.items() if t.n >= min_hands]
    z = family_z(len(tested))
    return {p for p in tested if ledger[p].upper(z) < 0}, z


def opponent_profiles(conn):
    """VPIP/PFR/hands per opponent from the live model (`models.v1`), as short reads."""
    row = conn.execute("SELECT value FROM kv WHERE key = 'models.v1'").fetchone()
    if not row:
        return {}
    out = {}
    for name, st in json.loads(row[0]).get("players", {}).items():
        hands = st.get("hands", 0)
        rate = lambda c: (c.get("hit", 0) / c["opp"]) if c and c.get("opp") else None
        vpip, pfr = rate(st.get("vpip")), rate(st.get("pfr"))
        if vpip is None or pfr is None:
            continue
        out[name] = f"{style_of(st, vpip, pfr, hands)} VPIP {vpip * 100:.0f}/PFR {pfr * 100:.0f}, {hands:.0f}h"
    return out


def style_of(st, vpip, pfr, hands):
    """The dashboard's style label (`api/opponents.rs` `style_of`), same names and thresholds, so the
    monitor and the scout view never call one player two things (0246: VPIP 34 read "tight" here). The
    aggression frequency is not in the model's counters, so its gates take the dashboard's defaults."""
    if hands < 20:
        return "Sampling"
    folds = st.get("fold_vs_bet") or []
    opp = sum(c.get("opp", 0) for c in folds)
    fold = sum(c.get("hit", 0) for c in folds) / opp if opp >= 12 else None
    passive_gap = vpip - pfr
    if vpip >= 0.45 and pfr >= 0.28:
        return "Maniac"
    if vpip >= 0.38 and passive_gap >= 0.2 and (fold is None or fold <= 0.35):
        return "Calling station"
    if fold is not None and fold >= 0.6:
        return "Overfolder"
    if fold is not None and fold <= 0.2:
        return "Sticky"
    if vpip <= 0.16 and pfr <= 0.12:
        return "Nit"
    if vpip <= 0.26 and pfr >= 0.16:
        return "Tight-aggressive"
    if vpip > 0.26 and pfr >= 0.2:
        return "Loose-aggressive"
    if passive_gap >= 0.15:
        return "Loose-passive"
    return "Balanced"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--interval", type=int, default=300)
    ap.add_argument("--summary-min", type=int, default=30)
    ap.add_argument("--big-loss-bb", type=float, default=100)
    ap.add_argument("--big-win-bb", type=float, default=None, help="default: same as --big-loss-bb")
    ap.add_argument("--min-hands", type=int, default=150)
    ap.add_argument("--stall-min", type=int, default=20)
    ap.add_argument("--active-hours", type=float, default=2, help="stall-watch only bots with a hand this recent")
    ap.add_argument("--once", action="store_true")
    a = ap.parse_args()
    if a.big_win_bb is None:
        a.big_win_bb = a.big_loss_bb

    conn = connect()
    fleet = fleet_names(conn)
    # Whole-history ledger per opponent: bb that moved between us and them per shared hand (0221).
    ledger = defaultdict(Tally)
    last = 0
    for rowid, bot, _, _, net, bb, players, winners, s, pot, treat in read_hands(conn, 0):
        last = rowid
        if net is not None:
            add_flows(ledger, fleet, bot, s, pot, winners, treat=treat)
    nemeses, _ = nemeses_of(ledger, a.min_hands)
    last_event = conn.execute("SELECT COALESCE(MAX(id), 0) FROM events").fetchone()[0]
    last_hand_at = {b: time.time() for b in active_bots(conn, a.active_hours)}
    stalled = set()
    window_bot = defaultdict(lambda: [0, 0])
    window_opp = defaultdict(float)
    window_seen = defaultdict(int)
    known = set(ledger)
    window_start = time.time()
    print(f"{now()} START monitor: {last} hands, {len(ledger)} opponents, "
          f"{len(nemeses)} beat us at 95%: {', '.join(sorted(nemeses)) or 'none'}", flush=True)

    while True:
        if a.once:
            window_start = 0
        else:
            time.sleep(a.interval)
        try:
            conn = connect()
            fleet |= fleet_names(conn)
            for rowid, bot, hand_id, ended, net, bb, players, winners, s, pot, treat in read_hands(conn, last):
                last = rowid
                last_hand_at[bot] = time.time()
                stalled.discard(bot)
                if net is None:
                    continue
                window_bot[bot][0] += 1
                window_bot[bot][1] += net
                # Per opponent, only the chips that moved between us and them: a table's result is
                # not credited to everyone dealt in (three players once read "-177bb" each for one
                # table's 40 hands).
                add_flows(ledger, fleet, bot, s, pot, winners, window_opp, treat=treat)
                for p in players:
                    if p not in fleet:
                        window_seen[p] += 1
                if net >= a.big_win_bb * bb:
                    print(f"{now()} BIGWIN {bot} {net:+d} chips ({net / bb:+.0f} bb) hand {hand_id}; against {', '.join(p for p in players if p not in fleet) or '?'}", flush=True)
                if net <= -a.big_loss_bb * bb:
                    print(f"{now()} BIGLOSS {bot} {net:+d} chips ({net / bb:+.0f} bb) hand {hand_id}; won by {winners or '?'}", flush=True)
            proven, z = nemeses_of(ledger, a.min_hands)
            for p in sorted(proven - nemeses):
                t = ledger[p]
                print(f"{now()} NEMESIS {p} beats us: {t.mean() * 100:+.1f} bb/100 moved to them over {t.n} champion hands "
                      f"(experiment-arm hands excluded, 0361; family-wise 95% upper {t.upper(z) * 100:+.1f}, z {z:.2f})", flush=True)
            nemeses |= proven
            for b, at in last_hand_at.items():
                if b not in stalled and time.time() - at > a.stall_min * 60:
                    stalled.add(b)
                    print(f"{now()} STALL {b}: no completed hand for {a.stall_min}+ minutes", flush=True)
            for eid, bot, msg in conn.execute("SELECT id, bot, message FROM events WHERE id > ? AND level = 'error' ORDER BY id LIMIT 20", (last_event,)):
                print(f"{now()} ERROR {bot}: {msg[:200]}", flush=True)
            last_event = conn.execute("SELECT COALESCE(MAX(id), 0) FROM events").fetchone()[0]
            if time.time() - window_start >= a.summary_min * 60:
                hands = sum(v[0] for v in window_bot.values())
                net = sum(v[1] for v in window_bot.values())
                bots = " ".join(f"{b} {v[1]:+d}/{v[0]}" for b, v in sorted(window_bot.items()))
                ranked = sorted(window_opp.items(), key=lambda kv: kv[1])
                takers = ", ".join(f"{p} {v:+.0f}bb" for p, v in ranked[:3] if v < 0) or "none"
                donors = ", ".join(f"{p} {v:+.0f}bb" for p, v in reversed(ranked[-3:]) if v > 0) or "none"
                season_worst = format_toughest(toughest(ledger, a.min_hands))
                print(f"{now()} SUMMARY {a.summary_min}m: {hands} hands {net:+d} chips | {bots} | took the most from us: {takers} | "
                      f"gave us the most: {donors} | toughest all-time (bb/100 moved between us and them in champion hands, ranked without each one's biggest pot): {season_worst}", flush=True)
                new = sorted(p for p in window_seen if p not in known)
                known |= set(window_seen)
                profiles = opponent_profiles(conn)
                busiest = sorted(window_seen.items(), key=lambda kv: -kv[1])[:4]
                reads = ", ".join(f"{p} ({profiles.get(p, 'no model')}; we {window_opp[p]:+.0f} bb against them over {n} hands)" for p, n in busiest) or "none"
                print(f"{now()} OPPONENTS {len(window_seen)} faced, {len(new)} new{': ' + ', '.join(new[:6]) if new else ''} | most played: {reads}", flush=True)
                window_bot.clear()
                window_opp.clear()
                window_seen.clear()
                window_start = time.time()
            conn.close()
        except sqlite3.Error as e:
            print(f"{now()} MONITOR db error: {e}", flush=True)
        if a.once:
            return


if __name__ == "__main__":
    main()
