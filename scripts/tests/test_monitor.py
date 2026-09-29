"""monitor.py: stall watch, toughest ranking, chip flow and family-wise nemesis verdicts."""
import datetime as dt
import importlib.util
import json
import sqlite3
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("monitor", Path(__file__).resolve().parent.parent / "monitor.py")
mon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mon)


class StallWatch(unittest.TestCase):
    def test_a_retired_name_is_not_watched_for_stalls(self):
        # 2026-09-23: SvanBotV7 was renamed SvanBotV10; its last hand stayed in `hands` and every
        # monitor restart reported it stalled 20 minutes later (0184).
        conn = sqlite3.connect(":memory:")
        conn.execute("CREATE TABLE hands (bot TEXT, ended_at TEXT)")
        conn.executemany("INSERT INTO hands VALUES (?, ?)", [
            ("SvanBotV7", "2026-09-22T22:54:28.044154435+00:00"),
            ("SvanBotV10", "2026-09-23T02:51:57.597454538+00:00"),
            ("Svanar", "2026-09-23T02:40:00+00:00"),
        ])
        at = dt.datetime(2026, 9, 23, 2, 55, tzinfo=dt.timezone.utc).timestamp()
        self.assertEqual(mon.active_bots(conn, 2, at), {"SvanBotV10", "Svanar"})
        self.assertEqual(mon.active_bots(conn, 6, at), {"SvanBotV10", "Svanar", "SvanBotV7"})


class Toughest(unittest.TestCase):
    def test_one_cooler_does_not_outrank_a_steady_loser(self):
        # 2026-09-24: one -5,255 bb pot made oj_carlton32 "toughest" at -604 bb/100 although we
        # won +42,740 in the other 515 hands with them (0209).
        cooler, steady = mon.Tally(), mon.Tally()
        cooler.add(-5255.0)
        for _ in range(515):
            cooler.add(4.0)
        for _ in range(516):
            steady.add(-1.0)
        worst = mon.toughest({"oj": cooler, "grinder": steady}, 150)
        self.assertEqual([p for _, p, _ in worst], ["grinder", "oj"])
        self.assertAlmostEqual(cooler.mean_without_biggest_loss(), 4.0)
        line = mon.format_toughest(worst)
        self.assertIn("grinder -100bb/100 (516)", line)
        self.assertIn("oj -619bb/100, +400 without its biggest pot (516)", line)

    def test_a_single_hand_has_no_trimmed_mean(self):
        t = mon.Tally()
        t.add(-3.0)
        self.assertEqual(t.mean_without_biggest_loss(), -3.0)


# The live hand `sv10_core::flow`'s tests use: hero (seat 4) shoves with an amount-less AllIn record,
# seat 5 raises to 4,629 and only 2,000 of it is called. Both implementations must agree on it.
ALL_IN = json.loads(r'''{"players":[[0,"MissCard"],[1,"jonnaBee"],[2,"Bertabot"],[3,"RObert"],[4,"SurSvan"],[5,"POKER_STUDY_AI"]],"button":2,"bb":20,"history":[{"seat":5,"street":"Preflop","kind":"Raise","to":50,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":true},{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":1,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":2,"street":"Preflop","kind":"Call","to":50,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":3,"street":"Preflop","kind":"Fold","to":0,"pot_before":130,"to_call_before":40,"bet_before":10,"full_raise":false},{"seat":4,"street":"Preflop","kind":"Call","to":50,"pot_before":130,"to_call_before":30,"bet_before":20,"full_raise":false},{"seat":4,"street":"Flop","kind":"AllIn","to":0,"pot_before":160,"to_call_before":0,"bet_before":0,"full_raise":false},{"seat":5,"street":"Flop","kind":"Raise","to":4629,"pot_before":2110,"to_call_before":1950,"bet_before":0,"full_raise":true},{"seat":2,"street":"Flop","kind":"Fold","to":0,"pot_before":6739,"to_call_before":4629,"bet_before":0,"full_raise":false}],"board":["5d","As","3s","6s","Th"],"shown":[[4,["Ts","Ac"]],[5,["Ah","5h"]]]}''')


class ChipFlow(unittest.TestCase):
    def test_all_in_amounts_come_from_the_pot_and_uncalled_chips_are_returned(self):
        c = mon.contributions(ALL_IN, 4060)
        self.assertEqual((c[4], c[5], c[2], c[3], c[0], c[1]), (2000, 2000, 50, 10, 0, 0))
        self.assertIsNone(mon.contributions(ALL_IN, 4061))

    def test_flow_charges_only_the_players_chips_moved_between(self):
        self.assertEqual(mon.flow_to_hero(ALL_IN, 4060, ["SurSvan"], 4), {0: 0, 1: 0, 2: 50, 3: 10, 5: 2000})
        lost = mon.flow_to_hero(ALL_IN, 4060, ["POKER_STUDY_AI"], 4)
        self.assertEqual((lost[5], lost[2], lost[3]), (-2000, 0.0, 0.0))
        split = mon.flow_to_hero(ALL_IN, 4060, ["SurSvan", "POKER_STUDY_AI"], 4)
        self.assertEqual((split[5], split[2], split[3]), (0.0, 25.0, 5.0))

    def test_the_folders_at_a_lost_pot_are_not_charged(self):
        # 2026-09-26: lionkingbig read -522 bb/100 "dealt in" while Bottelon2 won our pots at its
        # tables; flow charges only the winner.
        ledger = mon.defaultdict(mon.Tally)
        self.assertTrue(mon.add_flows(ledger, {"SurSvan"}, "SurSvan", ALL_IN, 4060, "POKER_STUDY_AI"))
        self.assertEqual(ledger["POKER_STUDY_AI"].s, -100.0)
        self.assertEqual(ledger["Bertabot"].s, 0.0)
        self.assertFalse(mon.add_flows(ledger, {"SurSvan"}, "SurSvan", ALL_IN, 4061, "POKER_STUDY_AI"))

    def test_the_window_charges_the_winner_not_the_whole_table(self):
        window = mon.defaultdict(float)
        ledger = mon.defaultdict(mon.Tally)
        self.assertTrue(mon.add_flows(ledger, {"SurSvan"}, "SurSvan", ALL_IN, 4060, "POKER_STUDY_AI", window))
        self.assertEqual(window["POKER_STUDY_AI"], -100.0)
        self.assertEqual(window["Bertabot"], 0.0)


class SidePotFlow(unittest.TestCase):
    @staticmethod
    def hand():
        return json.loads(r'''{"players":[[0,"hero"],[1,"short"],[2,"other"]],"button":0,"bb":20,"stacks":[[0,1000],[1,100],[2,1000]],"history":[{"seat":0,"street":"Preflop","kind":"AllIn","to":1000,"pot_before":30,"bet_before":0},{"seat":1,"street":"Preflop","kind":"AllIn","to":100,"pot_before":1030,"bet_before":10},{"seat":2,"street":"Preflop","kind":"AllIn","to":1000,"pot_before":1120,"bet_before":20}],"board":["2c","3d","7h","9s","Tc"],"shown":[[0,["Kh","Kc"]],[1,["Ah","Ac"]],[2,["Qh","Qc"]]]}''')

    def test_engine_side_pot_settlement_reconciles_for_each_seat(self):
        # Exact engine payouts [1800,300,0] on contributions [1000,100,1000].
        hand = self.hand()
        for hero, net in [(0, 800), (1, 200), (2, -1000)]:
            with self.subTest(hero=hero):
                flows = mon.flow_to_hero(hand, 2100, ['hero', 'short'], hero)
                self.assertEqual(sum(flows.values()), net)
        self.assertEqual(mon.flow_to_hero(hand, 2100, ['hero', 'short'], 0), {1: -100, 2: 900})

    def test_insufficient_side_pot_evidence_is_not_fabricated(self):
        hand = self.hand()
        hand['shown'] = []
        self.assertIsNone(mon.flow_to_hero(hand, 2100, ['hero', 'short'], 0))
        self.assertEqual(sum(mon.flow_to_hero(hand, 2100, ['hero'], 0).values()), 1100)

    def test_showdown_ranks_cover_all_categories_and_the_wheel(self):
        cases = [
            ('As Kd Qh 9c 7s 4d 2h', (0, 14, 13, 12, 9, 7)),
            ('As Ad Qh 9c 7s 4d 2h', (1, 14, 12, 9, 7)),
            ('As Ad Kh Kc 7s 4d 2h', (2, 14, 13, 7)),
            ('As Ad Ah 9c 7s 4d 2h', (3, 14, 9, 7)),
            ('As 2d 3h 4c 5s 9d Kh', (4, 5)),
            ('As Qs 9s 7s 2s Kd 4h', (5, 14, 12, 9, 7, 2)),
            ('As Ad Ah Kc Ks 4d 2h', (6, 14, 13)),
            ('As Ad Ah Ac Ks 4d 2h', (7, 14, 13)),
            ('As Ks Qs Js Ts 4d 2h', (8, 14)),
        ]
        for cards, expected in cases:
            with self.subTest(cards=cards):
                values = cards.split()
                self.assertEqual(mon.showdown_rank(values[:5], values[5:]), expected)
        self.assertIsNone(mon.showdown_rank(['As'] * 5, ['As', 'Kd']))
        self.assertIsNone(mon.showdown_rank(['As', '2d', '3h', '4c'], ['5s', 'Kh']))


class Nemesis(unittest.TestCase):
    def test_family_z_matches_the_rust_bound(self):
        self.assertAlmostEqual(mon.family_z(1), 1.96, places=2)
        self.assertAlmostEqual(mon.family_z(130), 3.55, places=2)

    def test_one_of_many_at_95_percent_is_not_a_nemesis(self):
        lucky, real = mon.Tally(), mon.Tally()
        for x in [4.0, -6.0] * 100:
            lucky.add(x)
        for x in [-40.0, -60.0] * 100:
            real.add(x)
        ledger = {"lucky": lucky, "real": real}
        self.assertEqual(mon.nemeses_of(ledger, 150)[0], {"lucky", "real"}, "two tested: 95% at z 2.24")
        ledger.update({f"donor{i}": mon.Tally() for i in range(128)})
        for t in list(ledger.values())[2:]:
            for x in [1.0, 3.0] * 100:
                t.add(x)
        self.assertEqual(mon.nemeses_of(ledger, 150)[0], {"real"})


if __name__ == "__main__":
    unittest.main()


class StyleLabels(unittest.TestCase):
    """0246: the monitor called VPIP 30-34 players "tight" while the scout view called them
    loose-aggressive; the labels now follow the dashboard's rules (api/opponents.rs `style_of`)."""

    @staticmethod
    def stats(fold=None):
        if fold is None:
            return {}
        return {"fold_vs_bet": [{"hit": fold * 100, "opp": 100}, {"hit": 0, "opp": 0}, {"hit": 0, "opp": 0}]}

    def test_labels_match_the_dashboard_rules(self):
        cases = [
            (self.stats(0.45), 0.30, 0.20, 500, "Loose-aggressive"),
            (self.stats(0.45), 0.24, 0.18, 500, "Tight-aggressive"),
            (self.stats(), 0.34, 0.18, 500, "Loose-passive"),
            (self.stats(0.30), 0.50, 0.10, 500, "Calling station"),
            (self.stats(0.65), 0.30, 0.16, 500, "Overfolder"),
            (self.stats(0.15), 0.30, 0.16, 500, "Sticky"),
            (self.stats(0.45), 0.12, 0.10, 500, "Nit"),
            (self.stats(), 0.50, 0.30, 500, "Maniac"),
            (self.stats(0.45), 0.28, 0.17, 500, "Balanced"),
            (self.stats(), 0.30, 0.20, 10, "Sampling"),
        ]
        for st, vpip, pfr, hands, want in cases:
            with self.subTest(vpip=vpip, pfr=pfr, want=want):
                self.assertEqual(mon.style_of(st, vpip, pfr, hands), want)
