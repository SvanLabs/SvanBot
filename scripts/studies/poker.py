#!/usr/bin/env python3
"""Describe frozen poker evidence; never open the input for writing.
Generated-by: codex/gpt-6
"""
import argparse
import collections
import fractions
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import statistics
import zlib


def clustered(rows, field):
    groups = collections.defaultdict(list)
    for row in rows:
        groups[row['bot'], row['hand_id']].append(row[field])
    values = [v for group in groups.values() for v in group]
    if not values:
        return {'n': 0}
    mean = statistics.mean(values)
    n, g = len(values), len(groups)
    se = math.sqrt(g / (g - 1) * sum(sum(v - mean for v in group) ** 2 for group in groups.values()) / n ** 2) if g > 1 else None
    return {'n': n, 'hand_clusters': g, 'mean': mean, 'ci95': [mean - 1.96 * se, mean + 1.96 * se] if se is not None else None}


def load(source):
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    c = sqlite3.connect(f'{source.resolve().as_uri()}?mode=ro&immutable=1', uri=True)
    c.row_factory = sqlite3.Row
    c.execute('PRAGMA query_only=ON')
    dictionaries = dict(c.execute('SELECT id, bytes FROM pack_dicts'))

    def decode(value):
        if isinstance(value, bytes):
            assert value[:3] == b'SVZ' and len(value) >= 16
            n = int.from_bytes(value[4:12], 'little')
            assert n <= 64 << 20
            decoder = zlib.decompressobj(-15, zdict=dictionaries[value[3]]) if value[3] else zlib.decompressobj(-15)
            raw = decoder.decompress(value[16:], n + 1) + decoder.flush()
            assert len(raw) == n and zlib.crc32(raw) == int.from_bytes(value[12:16], 'little')
            value = raw.decode()
        return json.loads(value)

    hands = {}
    for row in c.execute('SELECT * FROM hands'):
        h = dict(row)
        s = h['summary_obj'] = decode(h['summary'])
        h['bb'] = s['bb']
        seats = sorted(seat for seat, name in s['players'])
        hero, button = h['hero_seat'], s['button']
        order = sorted(seats, key=lambda seat: (seat - button - 1) % 6)
        if hero not in seats:
            h['position'] = 'unknown'
        elif len(seats) == 2:
            h['position'] = 'BTN' if hero == button else 'BB'
        else:
            i = order.index(hero)
            h['position'] = ['SB', 'BB'][i] if i < 2 else {0: 'BTN', 1: 'CO', 2: 'MP'}.get(len(seats) - 1 - i, 'EP')
        own = [a for a in s['history'] if a['seat'] == hero]
        h['last_street'] = own[-1]['street'] if own else 'no recorded action'
        h['last_action'] = own[-1]['kind'] if own else 'no recorded action'
        h['arm'] = 'ordinary'
        hands[h['bot'], h['hand_id']] = h
    for row in c.execute('SELECT * FROM hand_provenance'):
        hands[row['bot'], row['hand_id']]['arm'] = row['arm']
    decisions = []
    for row in c.execute('SELECT * FROM decisions ORDER BY id'):
        d = dict(row)
        d['detail_obj'] = decode(d['detail'])
        candidates = d['detail_obj'].get('candidates', [])
        chosen = [v for v in candidates if v['action'] == d['action'] and (d['action'] != 'raise' or v.get('amount') == d['amount'])]
        d['picked'] = chosen[0] if chosen else None
        d['arm'] = hands[d['bot'], d['hand_id']]['arm']
        decisions.append(d)
    calibration = [dict(row) for row in c.execute('SELECT * FROM calibration ORDER BY id')]
    lookup = collections.defaultdict(list)
    for d in decisions:
        if d['picked'] and d['picked'].get('category'):
            lookup[d['bot'], d['hand_id'], d['picked']['category']].append(d)
    for r in calibration:
        r['arm'] = hands[r['bot'], r['hand_id']]['arm']
        for d in list(lookup[r['bot'], r['hand_id'], r['category']]):
            chosen, bb = d['picked'], hands[r['bot'], r['hand_id']]['bb']
            if abs((chosen['ev'] - chosen.get('bias', 0)) / bb - r['predicted']) < 1e-7:
                r['decision_id'] = d['id']
                r['corrected_residual'] = r['realized'] - r['predicted'] - chosen.get('bias', 0) / bb
                lookup[r['bot'], r['hand_id'], r['category']].remove(d)
                break
    audits = [dict(row) for row in c.execute('SELECT * FROM decision_audit ORDER BY id')]
    c.close()
    return {'hmap': hands, 'ds': decisions, 'cal': calibration, 'audits': audits, 'clustered': clustered, 'out': {'source_sha256': digest}}


def flop(x):
    lookup = {d['id']: d for d in x['ds']}
    rows = [r.copy() for r in x['cal'] if r['category'] == 'flop:bet:big']
    grouped = collections.defaultdict(lambda: collections.defaultdict(list))
    matched = 0
    for r in rows:
        d = lookup[r['decision_id']]
        h = x['hmap'][r['bot'], r['hand_id']]
        summary, hero = h['summary_obj'], h['hero_seat']
        history = summary['history']
        groups = {
            'arm': r['arm'],
            'position': h['position'],
            'exact_size_to_pot': str(fractions.Fraction(d['amount'], max(d['pot'], 1))),
            'starting_hero_stack': '200bb_or_more' if dict(summary['stacks']).get(hero, 0) / h['bb'] >= 200 else 'under200bb',
            'later_action': h['last_street'] + ':' + h['last_action'],
            'split': 'early' if h['ended_at'] < '2026-09-29T13:00:00' else 'late',
        }
        indices = [i for i, a in enumerate(history) if a['street'] == 'Flop' and a['seat'] == hero
                   and a['kind'] == 'Raise' and a['to'] == d['amount']]
        if len(indices) == 1:
            matched += 1
            prefix = history[:indices[0]]
            folded = {a['seat'] for a in prefix if a['kind'] == 'Fold'}
            remaining = dict(summary['stacks'])
            posts = {}
            for a in history:
                if a['street'] == 'Preflop' and a['seat'] not in posts:
                    posts[a['seat']] = a['bet_before']
            for seat, amount in posts.items():
                remaining[seat] = remaining.get(seat, 0) - amount
            for a in prefix:
                remaining[a['seat']] = remaining.get(a['seat'], 0) - max(a['to'] - a['bet_before'], 0)
            opponents = [(seat, name) for seat, name in summary['players'] if seat != hero and seat not in folded]
            effective = min(remaining.get(hero, 0), max((remaining.get(seat, 0) for seat, name in opponents), default=0)) / h['bb']
            groups['reconstructed_effective_stack'] = '200bb_or_more' if effective >= 200 else 'under200bb'
            for seat, name in opponents:
                opponent = hashlib.sha256(name.encode()).hexdigest()[:12]
                grouped['active_opponent'][opponent].append(r)
        else:
            groups['reconstructed_effective_stack'] = 'ambiguous_action_match'
        for key, value in groups.items():
            grouped[key][str(value)].append(r)
    return {
        'Generated-by': 'codex/gpt-6',
        'source_sha256': x['out']['source_sha256'],
        'n': len(rows),
        'unique_action_matches': matched,
        'strata': {key: {group: clustered(rs, 'corrected_residual') for group, rs in sorted(groups.items())}
                   for key, groups in sorted(grouped.items())},
    }


def sizing(x):
    selected = []
    for d in x['ds']:
        if d['street'] in ['turn', 'river'] and d['action'] == 'raise' and d['picked']:
            h = x['hmap'][d['bot'], d['hand_id']]
            r = d.copy()
            r['gap'] = (max(c['ev'] for c in d['detail_obj']['candidates']) - d['picked']['ev']) / h['bb']
            r['split'] = 'early' if h['ended_at'] < '2026-09-29T13:00:00' else 'late'
            selected.append(r)
    audited = []
    for a in x['audits']:
        if a['street'] in ['turn', 'river'] and a['live_action'].startswith('raise'):
            r = a.copy()
            r['gap'] = max(a['gap_bb'], 0)
            r['arm'] = x['hmap'][a['bot'], a['hand_id']]['arm']
            audited.append(r)

    def describe(rows):
        groups = collections.defaultdict(list)
        for row in rows:
            groups[row['arm']].append(row)
        halves = collections.defaultdict(list)
        for row in rows:
            if 'split' in row:
                halves[row['split']].append(row)
        return {'all': clustered(rows, 'gap'),
                'arms': {k: clustered(v, 'gap') for k, v in sorted(groups.items())},
                'chronological_halves': {k: clustered(v, 'gap') for k, v in sorted(halves.items())}}

    return {'Generated-by': 'codex/gpt-6', 'source_sha256': x['out']['source_sha256'],
            'all_recorded_raise_live_price_gaps': {street: describe([r for r in selected if r['street'] == street])
                                                  for street in ['turn', 'river']},
            'selected_deep_audit_gaps': {street: describe([r for r in audited if r['street'] == street])
                                        for street in ['turn', 'river']}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path, help='closed immutable SQLite cohort copy')
    parser.add_argument('--study', choices=['flop', 'sizing'], default='flop')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expected-sha256', required=True)
    args = parser.parse_args()
    if args.source.resolve() == args.output.resolve() or (args.output.exists() and args.source.samefile(args.output)):
        parser.error('output must differ from the source database')
    assert hashlib.sha256(args.source.read_bytes()).hexdigest() == args.expected_sha256, 'cohort digest differs'
    report = {'flop': flop, 'sizing': sizing}[args.study](load(args.source))
    args.output.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
