import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Award, Crown, Flame, Medal, Sparkles, Swords, Trophy, TrendingDown, TrendingUp, X, Zap } from 'lucide-react';
import type { Bot, Decision, SeasonScope, Training } from './types';
import { request, SESSION_EVENT, StaleNote, usePoll } from './api';
import { fmt, sgn, share, time, SUITS } from './format';
import { NamesIn, PlayerName } from './playername';

const suit = SUITS;
const BOT_COLORS = ['#e4b956', '#6daa98', '#b68578', '#8fa2d8', '#c79bd6'];

export function MiniCards({ cards }: { cards: string[] }) {
  return <span className="mini-cards">{cards.map((c, i) => <span key={i} className={`mini-card ${'hd'.includes(c[1]) ? 'red' : ''}`}>{c[0]}{suit[c[1]]}</span>)}</span>;
}

interface LbRow {
  rank: number | null; name: string; score: number | null; hands: number | null;
  win_rate: number | null; pro: boolean | null; ours: boolean; rank_delta: number | null;
  gap_to_first: number | null; gap_to_next: number | null; gap_to_four: number | null;
  score_delta: number | null; score_velocity_per_hour: number | null;
  hands_delta: number | null; hands_velocity_per_hour: number | null;
}
interface Lb {
  entries: LbRow[];
  season: { season_number?: number; time_remaining_seconds?: number } | null;
  updated: number | null;
  stale: boolean;
  error: string | null;
}

function useLeaderboard(ms: number) {
  const [data, setData] = useState<Lb>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const next = await request<Lb>('/leaderboard');
        if (!alive) return;
        setData(next);
        setError(next.error || undefined);
      } catch {
        if (alive) setError('Leaderboard refresh failed');
      } finally {
        if (alive) setLoading(false);
      }
    };
    load();
    const id = setInterval(load, ms);
    window.addEventListener(SESSION_EVENT, load);
    return () => { alive = false; clearInterval(id); window.removeEventListener(SESSION_EVENT, load); };
  }, [ms]);
  return {data, loading, error};
}

export function SeasonRace() {
  const {data: lb, loading, error} = useLeaderboard(60000);
  const statePoll = usePoll<{training?: Training}>('/state', 60000);
  const state = statePoll.data;
  const fleetPoll = usePoll<FleetSeries>('/fleet', 60000);
  const fleet = fleetPoll.data;
  const [showAll, setShowAll] = useState(false);
  const rows = lb?.entries || [];
  const ours = rows.filter(r => r.ours);
  const best = [...ours].sort((a,b) => (a.rank ?? Number.MAX_SAFE_INTEGER) - (b.rank ?? Number.MAX_SAFE_INTEGER))[0];
  const visible = showAll ? rows.slice(0, 50) : rows.filter((r, i) => i < 5 || r.ours);
  const left = lb?.season?.time_remaining_seconds;
  const days = left != null ? Math.floor(left / 86400) : null;
  const hours = left != null ? Math.floor((left % 86400) / 3600) : null;
  const maxScore = Math.max(1, ...rows.flatMap(r => r.score == null ? [] : [r.score]));
  const stale = !!(lb?.stale || error);
  const fleetBot = fleet?.bots.find(bot => bot.name === best?.name);
  const recentFleet = fleetBot?.points.filter(point => point.ts >= Date.now() / 1000 - 86400) || [];
  const fleetStart = recentFleet.length >= 2 ? recentFleet[0] : undefined;
  const fleetEnd = recentFleet.length >= 2 ? recentFleet[recentFleet.length - 1] : undefined;
  const fleetHours = fleetStart && fleetEnd ? (fleetEnd.ts - fleetStart.ts) / 3600 : 0;
  const chipsPerHour = fleetStart && fleetEnd && fleetHours >= 5 / 60 ? (fleetEnd.total - fleetStart.total) / fleetHours : null;
  const freshness = loading && !lb ? 'Loading leaderboard' : stale ? `Leaderboard data stale${lb?.updated != null ? ` · last update ${time(lb.updated)}` : ''}` : lb?.updated != null ? `Updated ${time(lb.updated)}` : 'Leaderboard live';
  return <div className="race">
    <StaleNote poll={statePoll} />
    <StaleNote poll={fleetPoll} />
    <div className={`race-hero ${stale ? 'stale' : ''}`}>
      <Crown size={22} />
      <div>
        <span className="eyebrow">{lb?.season?.season_number != null ? `SEASON ${lb.season.season_number} · ` : ''}{days != null ? `${days}d ${hours}h LEFT` : 'TIME UNAVAILABLE'}</span>
        <strong>{loading && !lb ? 'Loading leaderboard' : best?.rank != null ? <><PlayerName name={best.name}/> is #{best.rank}</> : 'Rank unavailable'}</strong>
        <small>{best ? best.gap_to_first == null ? 'Gap to #1 unavailable' : `${fmt(best.gap_to_first)} chips behind #1 · ${ours.length} bots ranked` : 'Gap to #1 unavailable'}</small>
        <span className="race-freshness">{freshness}</span>
      </div>
    </div>
    <div className="race-targets" aria-label="Leaderboard targets">
      <div><span>BEST FLEET RANK</span><b>{best?.rank == null ? 'Unavailable' : `#${best.rank}`}</b></div>
      <div><span>GAP TO #1</span><b>{best?.gap_to_first == null ? 'Unavailable' : fmt(best.gap_to_first)}</b></div>
      <div><span>GAP TO #4</span><b>{best?.gap_to_four == null ? 'Unavailable' : fmt(best.gap_to_four)}</b></div>
      <div><span>GAP TO NEXT</span><b>{best?.gap_to_next == null ? 'Unavailable' : fmt(best.gap_to_next)}</b></div>
      <div><span>RECENT ESTIMATE</span><b>{best?.score_velocity_per_hour == null ? 'Unavailable' : `${sgn(best.score_velocity_per_hour)} / hr`}</b><small>Short-window estimate of leaderboard score · variance can be high</small></div>
      <div><span>CHIPS / HOUR</span><b>{chipsPerHour == null ? 'Unavailable' : `${sgn(chipsPerHour)} / hr`}</b><small>Recorded table net over the last 24 hours</small></div>
      <div><span>HANDS / HOUR</span><b>{best?.hands_velocity_per_hour == null ? 'Unavailable' : `${fmt(best.hands_velocity_per_hour, 1)} / hr`}</b><small>Derived over the same refresh window</small></div>
      <div><span>ACTIVE STRATEGY</span><b>{state?.training?.leaderboard_evidence?.strategy || 'Unavailable'}</b></div>
      <div><span>RESPONSE MODEL</span><b title={state?.training?.leaderboard_evidence?.neural}>{shortDigest(state?.training?.leaderboard_evidence?.neural) || 'Unavailable'}</b><small>Digest or stat fallback</small></div>
      <div><span>RANGE · CALIBRATION</span><b>{state?.training?.leaderboard_evidence ? `${stamped(state.training.leaderboard_evidence.range)} · ${stamped(state.training.leaderboard_evidence.calibration)}` : 'Unavailable'}</b></div>
      <div className="race-wide"><span>STRONGEST EVIDENCED OPPORTUNITY</span><b>{state?.training?.leaderboard_evidence?.opportunity || 'No validated opportunity'}</b>{state?.training?.leaderboard_evidence?.next_candidate && <small>Next candidate · {state.training.leaderboard_evidence.next_candidate} · evidence pending</small>}</div>
    </div>
    <div className="race-list">
      {visible.map(r => <div key={r.name} className={`race-row ${r.ours ? 'ours' : ''}`}>
        <span className="race-rank">{r.rank != null && r.rank <= 3 ? <Medal size={13} className={`medal-${r.rank}`} /> : r.rank == null ? '—' : `#${r.rank}`}</span>
        <span className="race-name"><PlayerName name={r.name}/>{r.ours && <em title="Our bot">OURS</em>}</span>
        <span className={`race-delta ${r.rank_delta && r.rank_delta > 0 ? 'up' : r.rank_delta && r.rank_delta < 0 ? 'down' : ''}`}>
          {r.rank_delta ? (r.rank_delta > 0 ? <TrendingUp size={11} /> : <TrendingDown size={11} />) : null}{r.rank_delta ? Math.abs(r.rank_delta) : ''}
        </span>
        <span className="race-score" title="Season score: off-table chips + chips at tables − rebuy penalties. This is not poker profit.">{fmt(r.score)} pts</span>
        <span className="race-bar"><i style={{ width: `${r.score == null ? 0 : Math.max(1, r.score / maxScore * 100)}%` }} /></span>
      </div>)}
    </div>
    <button className="text-button race-toggle" onClick={() => setShowAll(!showAll)}>{showAll ? 'Show top 5 + our bots' : 'Show top 50'}</button>
  </div>;
}

/** How one priced option reads: `Raise 168`, `Check`, `Call`, `Fold`, `All-in`. */
export function moveLabel(c: { action: string; amount?: number | null }) {
  const verb = c.action === 'all_in' ? 'All-in' : c.action[0].toUpperCase() + c.action.slice(1).replace('_', ' ');
  // A check or fold carries amount 0 (the server fills missing amounts with 0), so only a priced
  // bet names its size.
  return (c.action === 'raise' || c.action === 'all_in') && c.amount ? `${verb} ${fmt(c.amount)}` : verb;
}

/** The option the policy took, by the same rule the server uses to line a decision up with its own
 *  candidate table (`client/decide.rs`): the action, and for a raise the exact amount. */
export function chosenOption(decision?: Decision | null) {
  const cands = decision?.candidates ?? [];
  const index = decision ? cands.findIndex(c => c.action === decision.action && (c.action !== 'raise' || (c.amount ?? 0) === (decision.amount ?? 0))) : -1;
  return { cands, index, chosen: index >= 0 ? cands[index] : undefined };
}

/** The last decision as a sentence (0299), built from the record's own fields: the server's `reason`
 *  is shorthand followed by the option dump the chart now draws. Anything the record does not carry
 *  is left out rather than guessed, and a record with no option table falls back to its own words. */
export function planSentence(decision?: Decision | null): { text: string; raw: string } | null {
  if (!decision) return null;
  const raw = (decision.reason ?? '').split(' — ')[0].trim();
  const { cands, chosen } = chosenOption(decision);
  const best = cands.length ? Math.max(...cands.map(c => c.ev)) : null;
  const bestOption = best == null ? undefined : cands.find(c => c.ev === best);
  const heads = decision.opponent_models?.length ?? 0;
  const eq = decision.equity?.value;
  const price = decision.pot_odds;
  const clauses: string[] = [];
  const lead = [decision.street ? decision.street[0].toUpperCase() + decision.street.slice(1) : '', decision.hand_category?.toLowerCase() ?? ''].filter(Boolean).join(', ');
  if (eq != null || lead) {
    clauses.push(`${lead ? `${lead}: ` : ''}equity ${eq == null ? 'unreported' : share(eq)}${heads ? ` against ${heads} opponent${heads === 1 ? '' : 's'}` : ''}`);
  }
  if ((decision.to_call ?? 0) > 0 && price != null && eq != null) {
    clauses.push(`a ${fmt(decision.to_call)}-chip call needs ${share(price)} and this hand has ${share(eq)}`);
  }
  // Prose keeps the verb: `the policy calls`, `the policy raises to 75`.
  const verbs: Record<string, string> = { fold: 'folds', check: 'checks', call: 'calls', raise: 'raises', all_in: 'goes all in' };
  const verb = verbs[decision.action] ?? decision.action.replace('_', ' ');
  const move = (decision.action === 'raise' || decision.action === 'all_in') && decision.amount ? `${verb} to ${fmt(decision.amount)}` : verb;
  if (chosen && best != null && bestOption) {
    const margin = best - chosen.ev;
    clauses.push(`the policy ${move} — ${sgn(chosen.ev)} chips${margin <= Math.abs(best) * 0.001 ? `, the best of the ${cands.length} options it priced` : `, ${fmt(margin)} behind the best option (${moveLabel(bestOption)} at ${sgn(best)})`}`);
  } else if (raw) {
    clauses.push(`the policy ${move} — ${raw}`);
  } else {
    clauses.push(`the policy ${move}`);
  }
  const chosenBet = chosen && (chosen.action === 'raise' || chosen.action === 'all_in') ? chosen : undefined;
  const biggest = cands.filter(c => (c.fold_prob ?? 0) > 0).sort((a, b) => (b.fold_prob ?? 0) - (a.fold_prob ?? 0))[0];
  if (chosenBet?.fold_prob) clauses.push(`that bet folds them ${share(chosenBet.fold_prob)} of the time`);
  else if (biggest?.fold_prob) clauses.push(`the largest bet it priced, ${moveLabel(biggest)}, folds them ${share(biggest.fold_prob)} of the time`);
  if (!clauses.length) return raw ? { text: raw, raw } : null;
  return { text: `${clauses.join('; ')}.`, raw };
}

export function EvBars({ decision }: { decision?: Decision | null }) {
  const [hover, setHover] = useState<number | null>(null);
  const { cands, index: chosen } = chosenOption(decision);
  if (!cands.length) return null;
  const max = Math.max(1, ...cands.map(c => Math.abs(c.ev)));
  const best = Math.max(...cands.map(c => c.ev));
  const anyFold = cands.some(c => (c.fold_prob ?? 0) > 0);
  return <div className="ev-bars" aria-label="Expected value of every option considered">
    <div className="ev-title"><Sparkles size={12} /> WHY THIS MOVE · EXPECTED CHIPS PER OPTION</div>
    <div className="ev-head" aria-hidden="true"><span>OPTION</span><span/><span>EV</span><span>THEY FOLD</span></div>
    {cands.map((c, i) => <div key={i} className={`ev-row ${c.ev === best ? 'best' : ''} ${i === chosen ? 'chosen' : ''}`} onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
      <span className="ev-label">{moveLabel(c)}{i === chosen && <>{' '}<em className="ev-chosen">CHOSEN</em></>}</span>
      <span className="ev-track"><span className="ev-zero" />
        <i className={c.ev >= 0 ? 'pos' : 'neg'} style={{ width: `${Math.abs(c.ev) / max * 50}%`, [c.ev >= 0 ? 'left' : 'right']: '50%' } as React.CSSProperties} />
      </span>
      <span className="ev-value">{sgn(c.ev, 0)}</span>
      <span className="ev-fold">{c.fold_prob ? share(c.fold_prob) : '—'}</span>
      {hover === i && <span className="ev-tip">{c.action === 'fold' ? 'Folding risks nothing more.' : `${c.fold_prob ? `Opponents fold ${share(c.fold_prob)}` : ''}${c.fold_prob && c.equity_called != null ? ' · ' : ''}${c.equity_called != null ? `equity if called ${share(c.equity_called)}` : ''}`}</span>}
    </div>)}
    {decision && chosen < 0 && <p className="footnote">The move it took ({moveLabel(decision)}) is not among the {cands.length} options priced in this record.</p>}
    {anyFold && <p className="footnote">“They fold” is the chance every opponent still in the hand folds to that bet, from the same response model the EV uses (flop / turn / river); a check or call has none.</p>}
  </div>;
}

interface FleetSeries { bots: { name: string; hands: number; total: number; points: { hand: number; ts: number; total: number }[]; all_time: { hands: number; total: number } }[]; season: SeasonScope }

/** A model digest cut to 8 hex characters (the full value stays in the tooltip). */
function shortDigest(v?: string) {
  return v && /^[0-9a-f]{12,}$/.test(v) ? v.slice(0, 8) : v;
}

/** `fitted@<unix>` / `updated@<unix>` from the API, as a readable age. */
function stamped(v?: string) {
  const m = v?.match(/^(\w+)@(\d+)$/);
  if (!m) return v ?? '';
  const mins = Math.max(0, Math.round((Date.now() / 1000 - Number(m[2])) / 60));
  const age = mins < 1 ? 'just now' : mins < 60 ? `${mins} min ago` : mins < 2880 ? `${Math.floor(mins / 60)} h ago` : `${Math.floor(mins / 1440)} d ago`;
  return `${m[1]} ${age}`;
}

/** What a season-scoped panel's figures cover, for the label next to them. */
function scopeLabel(scope?: SeasonScope) {
  return scope?.scoped ? `SEASON ${scope.number ?? '—'}` : 'ALL STORED HANDS';
}

export function FleetRace() {
  const dataPoll = usePoll<FleetSeries>('/fleet', 30000);
  const data = dataPoll.data;
  const [hover, setHover] = useState<{ x: number; name: string; total: number; hand: number } | null>(null);
  const [hidden, setHidden] = useState<Record<string, boolean>>({});
  const bots = data?.bots || [];
  const maxHands = Math.max(1, ...bots.map(b => b.hands));
  const vals = bots.flatMap(b => b.points.map(p => p.total));
  const lo = Math.min(0, ...vals), hi = Math.max(1, ...vals);
  const W = 560, H = 190;
  const x = (hand: number) => 10 + hand / maxHands * (W - 20);
  const y = (v: number) => H - 12 - (v - lo) / (hi - lo) * (H - 28);
  const fleetTotal = bots.reduce((s, b) => s + b.total, 0);
  const lifetimeTotal = bots.reduce((s, b) => s + (b.all_time?.total ?? 0), 0);
  return <div className="fleet-race">
    <StaleNote poll={dataPoll} />
    <div className="fleet-race-head"><span><Swords size={13} /> FLEET TOTAL · {scopeLabel(data?.season)} <b className={fleetTotal < 0 ? 'negative' : 'positive'}>{sgn(fleetTotal)}</b> chips{data?.season?.scoped && <small className="subtle"> · lifetime {sgn(lifetimeTotal)}</small>}</span>
      <span className="fleet-legend">{bots.map((b, i) => <button key={b.name} className={hidden[b.name] ? 'off' : ''} onClick={() => setHidden({ ...hidden, [b.name]: !hidden[b.name] })}><i style={{ background: BOT_COLORS[i % 5] }} />{b.name} <small>{sgn(b.total)}</small></button>)}</span></div>
    <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Cumulative profit race for every bot" onMouseLeave={() => setHover(null)}>
      <line x1="10" x2={W - 10} y1={y(0)} y2={y(0)} stroke="var(--line)" strokeDasharray="4 5" />
      {bots.map((b, i) => hidden[b.name] ? null : <polyline key={b.name} fill="none" stroke={BOT_COLORS[i % 5]} strokeWidth="2" points={b.points.map(p => `${x(p.hand)},${y(p.total)}`).join(' ')}
        onMouseMove={e => {
          const rect = (e.currentTarget.ownerSVGElement as SVGSVGElement).getBoundingClientRect();
          const hand = Math.round(((e.clientX - rect.left) / rect.width * W - 10) / (W - 20) * maxHands);
          const p = b.points.reduce((a, c) => Math.abs(c.hand - hand) < Math.abs(a.hand - hand) ? c : a, b.points[0]);
          setHover({ x: x(p.hand), name: b.name, total: p.total, hand: p.hand });
        }} />)}
      {hover && <g><line x1={hover.x} x2={hover.x} y1="8" y2={H - 8} stroke="#e4b95655" /><text x={Math.min(hover.x + 6, W - 150)} y="20" fill="#f2e3b6" fontSize="11">{hover.name} · hand {fmt(hover.hand)} · {sgn(hover.total)}</text></g>}
    </svg>
  </div>;
}

interface HandLite { slot: number; bot: string; hand_id: string; hole: string[]; board: string[]; net: number; pot?: number; ts: number; category?: string }
interface Highlights { fleet_total: number; fleet_hands: number; season: SeasonScope; biggest_wins: HandLite[]; biggest_losses: HandLite[]; monsters: HandLite[]; slams: HandLite[]; best_streak: { length: number; bot: string }; milestones: { id: string; title: string; detail: string; unlocked: boolean }[] }

export function HighlightsPanel({ onReplay }: { onReplay: (slot: number, handId: string) => void }) {
  const hPoll = usePoll<Highlights>('/highlights', 60000);
  const h = hPoll.data;
  const [tab, setTab] = useState<'wins' | 'monsters' | 'slams' | 'losses'>('wins');
  const list = tab === 'wins' ? h?.biggest_wins : tab === 'monsters' ? h?.monsters : tab === 'slams' ? h?.slams : h?.biggest_losses;
  const unlocked = h?.milestones.filter(m => m.unlocked).length || 0;
  return <div className="highlights">
    <StaleNote poll={hPoll} />
    <div className="badges">{h?.milestones.map(m => <span key={m.id} className={`badge ${m.unlocked ? 'on' : ''}`} title={`${m.title}: ${m.detail}`}>{m.unlocked ? <Award size={13} /> : <Trophy size={13} />}<b>{m.title}</b></span>)}</div>
    <p className="footnote">{unlocked} of {h?.milestones.length || 0} achievements unlocked across every season · {scopeLabel(h?.season).toLowerCase()}: {sgn(h?.fleet_total || 0)} chips over {fmt(h?.fleet_hands || 0)} hands, best streak {h?.best_streak.length || 0} winning hands{h?.best_streak.bot ? ` (${h.best_streak.bot})` : ''}</p>
    <div className="position-tabs">{(['wins', 'monsters', 'slams', 'losses'] as const).map(t => <button key={t} className={tab === t ? 'active' : ''} onClick={() => setTab(t)}>{t === 'wins' ? 'Wins' : t === 'monsters' ? 'Monsters' : t === 'slams' ? 'Slams' : 'Bad beats'}</button>)}</div>
    <div className="highlight-list">{(list || []).map(x => <button key={`${x.bot}-${x.hand_id}`} className="highlight" onClick={() => onReplay(x.slot, x.hand_id)}>
      <span className="highlight-icon">{tab === 'wins' ? <Flame size={14} /> : tab === 'monsters' ? <Zap size={14} /> : tab === 'slams' ? <Crown size={14} /> : <TrendingDown size={14} />}</span>
      <span className="highlight-main"><b><PlayerName nested name={x.bot}/></b><small>{x.category || time(x.ts)}</small></span>
      <MiniCards cards={x.hole} /><MiniCards cards={x.board} />
      <span className={x.net < 0 ? 'negative' : 'positive'}>{sgn(x.net)}</span>
    </button>)}{!list?.length && <p className="footnote">No hands here yet.</p>}</div>
  </div>;
}

interface Toast { id: number; bot: string; net: number }

export function WinToasts({ bots }: { bots: Bot[] }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const last = useRef<Record<number, number>>({});
  const counter = useRef(0);
  useEffect(() => {
    for (const b of bots) {
      const prev = last.current[b.slot];
      const now = b.metrics?.net_chips ?? 0;
      if (prev != null && now - prev >= 1000) {
        const id = ++counter.current;
        setToasts(t => [...t.slice(-3), { id, bot: b.name, net: now - prev }]);
        setTimeout(() => setToasts(t => t.filter(x => x.id !== id)), 6000);
      }
      last.current[b.slot] = now;
    }
  }, [bots]);
  return <div className="toasts" aria-live="polite">{toasts.map(t => <div key={t.id} className="toast"><Trophy size={16} /><span><b><PlayerName name={t.bot}/></b> scooped a pot</span><strong>{sgn(t.net)}</strong><button aria-label="Dismiss" onClick={() => setToasts(x => x.filter(y => y.id !== t.id))}><X size={12} /></button></div>)}</div>;
}

export function useAutoPlay(length: number, step: number, setStep: (n: number) => void) {
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  useEffect(() => {
    if (!playing) return;
    if (step >= length - 1) { setPlaying(false); return; }
    const id = setTimeout(() => setStep(step + 1), 900 / speed);
    return () => clearTimeout(id);
  }, [playing, step, length, speed]);
  return useMemo(() => ({ playing, setPlaying, speed, setSpeed }), [playing, speed]);
}

interface LiveEvent { type: string; ts: number; slot: number; bot: string; seat?: number; name?: string; action?: string; amount?: number | null; street?: string; pot?: number; cards?: string[]; net?: number | null; winners?: string[]; equity?: number | null; latency_ms?: number; hand_id?: string }

export function ActionTicker({ selectedSlot }: { selectedSlot?: number }) {
  const [events, setEvents] = useState<LiveEvent[]>([]);
  const [all, setAll] = useState(false);
  useEffect(() => {
    const onEvent = (e: Event) => {
      const ev = (e as CustomEvent<LiveEvent>).detail;
      setEvents(list => [ev, ...list].slice(0, 80));
    };
    window.addEventListener('sv-live', onEvent);
    return () => window.removeEventListener('sv-live', onEvent);
  }, []);
  const shown = events.filter(e => all || e.slot === selectedSlot).slice(0, 18);
  const describe = (e: LiveEvent) => {
    switch (e.type) {
      case 'action': return <><b><PlayerName name={e.name}/></b> {e.action?.replace('_', ' ')}{e.amount ? <> to <b>{fmt(e.amount)}</b></> : null}</>;
      case 'board': return <>{String(e.street).toUpperCase()} <MiniCards cards={e.cards || []} /></>;
      case 'decision': return <><b><PlayerName name={e.bot}/></b> decided <b>{e.action?.replace('_', ' ')}</b>{e.amount ? ` ${fmt(e.amount)}` : ''}{e.equity == null ? ' · no equity measurement' : ` · equity ${Math.round(e.equity * 100)}%`} · {fmt(e.latency_ms, 1)} ms</>;
      case 'result': return <><b><PlayerName name={e.bot}/></b> {e.net == null ? 'hand over' : e.net > 0 ? 'won' : e.net < 0 ? 'lost' : 'broke even'} <b className={(e.net || 0) < 0 ? 'negative' : 'positive'}>{e.net ? sgn(e.net) : ''}</b>{e.winners?.length ? ` · winner ${e.winners.join(', ')}` : ''}</>;
      default: return e.type;
    }
  };
  return <div className="ticker">
    <div className="ticker-head"><span className="live-label"><span className="status-dot" />STREAMING</span>
      <span className="ticker-toggle"><button className={!all ? 'active' : ''} onClick={() => setAll(false)}>This bot</button><button className={all ? 'active' : ''} onClick={() => setAll(true)}>All bots</button></span></div>
    <div className="ticker-list">{shown.map((e, i) => <div key={`${e.ts}-${i}-${e.type}`} className={`tick tick-${e.type} ${e.action ? `act-${e.action}` : ''}`}>
      <time>{time(e.ts, { seconds: true })}</time>
      {all && <span className="tick-bot"><PlayerName name={e.bot}/></span>}
      <span className="tick-text">{describe(e)}</span>
    </div>)}{!shown.length && <p className="footnote">Waiting for the next action at the table…</p>}</div>
  </div>;
}

interface CalRow { category: string; n: number; predicted_bb: number; realized_bb: number; residual_bb: number; se_bb: number; bias_bb: number; residual_pot?: number; se_pot?: number; pot_bias_candidate?: number; bound_by?: string }

interface StreetFit { n: number; predicted: number; actual: number; held_out_gain: number; held_out_lower: number; active: boolean; shift: number }
interface FoldFit { shift: number[]; streets: StreetFit[] }
interface JamFit { n: number; train_shift: number; held_out_saved: number; held_out_lower: number; active: boolean; shift: number }
interface CalData { rows: CalRow[]; active_corrections: number; fold?: FoldFit | null; river_jam?: JamFit | null; live?: { fold_logit_shift: number[]; river_jam_call_shift: number; preflop_fold_logit_shift?: number; deep_call_shift?: number; overbet_call_shift?: number; overbet_call_slope?: number } }

/** Live-fitted corrections (0156, 0159): what was measured, whether it passed its held-out check,
 * and whether the running policy uses it. */
function LiveFits({ data }: { data: CalData }) {
  const fold = data.fold, jam = data.river_jam, live = data.live;
  const same = (a: number, b: number) => Math.abs(a - b) < 1e-9;
  return <>
    <p className="footnote">Fold prediction by street: our heads-up bets, predicted vs actual fold rate. A logit shift is used only while it beats the raw prediction on newer hands.</p>
    <div className="calib-list">{(fold?.streets || []).map((s, i) => <div key={i} className="calib-row">
      <span className="calib-cat">{['Flop', 'Turn', 'River'][i]} folds<small>{fmt(s.n)} bets</small></span>
      <span className="calib-vals"><span>model {(s.predicted * 100).toFixed(1)}%</span><span className={s.actual < s.predicted ? 'negative' : 'positive'}>real {(s.actual * 100).toFixed(1)}%</span></span>
      <span className={`calib-bias ${s.active ? 'on' : ''}`} title={`held-out gain ${sgn(s.held_out_gain * 1000, 1)} mnats, 95% lower ${sgn(s.held_out_lower * 1000, 1)}${live && !same(live.fold_logit_shift[i] ?? 0, s.shift) ? ' · not yet in live play' : ''}`}>{s.active ? `shift ${sgn(s.shift, 2)}` : 'not installed'}</span>
    </div>)}{!fold && <p className="footnote">The learner fits this each cycle.</p>}</div>
    <p className="footnote">River all-in calls: our estimated equity against the exact equity versus the shown hand. The correction is used only once folding the calls it flips has saved chips on newer hands.</p>
    {jam ? <div className="calib-list"><div className="calib-row">
      <span className="calib-cat" title={`estimate minus exact equity on the older half: ${sgn(jam.train_shift, 3)}`}>River jam calls<small>{fmt(jam.n)} calls · over {sgn(jam.train_shift, 2)}</small></span>
      <span className="calib-vals" title="chips per newer call saved by folding the calls the shift flips, and its 95% lower bound"><span>saves {sgn(jam.held_out_saved, 0)}</span><span className={jam.held_out_lower > 0 ? 'positive' : 'negative'}>low {sgn(jam.held_out_lower, 0)}</span></span>
      <span className={`calib-bias ${jam.active ? 'on' : ''}`} title={live && !same(live.river_jam_call_shift, jam.shift) ? 'not yet in live play' : 'in live play as shown'}>{jam.active ? `shift −${jam.shift.toFixed(3)}` : 'not installed'}</span>
    </div></div> : <p className="footnote">The learner fits this each cycle.</p>}
    {live && <>
      <p className="footnote">All-in calls in deep pots and against overbets: the equity haircut in live play (the largest one that applies). Against an overbet it grows with the size of the shove once the size-scaled fit passes its held-out check (0233).</p>
      <div className="calib-list">
        <div className="calib-row"><span className="calib-cat">Deep pots<small>500+ bb</small></span><span className="calib-vals"/><span className={`calib-bias ${live.deep_call_shift ? 'on' : ''}`}>{live.deep_call_shift ? `shift −${live.deep_call_shift.toFixed(3)}` : 'not installed'}</span></div>
        <div className="calib-row"><span className="calib-cat">Overbet shoves<small>1.5x pot or more</small></span><span className="calib-vals"/><span className={`calib-bias ${live.overbet_call_shift ? 'on' : ''}`}>{live.overbet_call_shift ? `shift −${live.overbet_call_shift.toFixed(3)}` : 'not installed'}</span></div>
        <div className="calib-row"><span className="calib-cat">Overbet by size<small>grows with the shove</small></span>
          <span className="calib-vals">{live.overbet_call_slope ? <><span>40x −{overbetShift(live.overbet_call_slope, 40).toFixed(3)}</span><span>180x −{overbetShift(live.overbet_call_slope, 180).toFixed(3)}</span></> : null}</span>
          <span className={`calib-bias ${live.overbet_call_slope ? 'on' : ''}`}>{live.overbet_call_slope ? `slope ${live.overbet_call_slope.toFixed(3)}` : 'not installed'}</span></div>
      </div>
    </>}
  </>;
}

/** The size-scaled overbet haircut at a shove of `ratio` times the pot (mirrors `policy::call_equity`). */
function overbetShift(slope: number, ratio: number): number {
  return ratio < 1.5 ? 0 : Math.min(0.4, slope * (1 + Math.log(ratio / 1.5)));
}

export function CalibrationPanel() {
  const dataPoll = usePoll<CalData>('/calibration', 60000);
  const data = dataPoll.data;
  const rows = data?.rows || [];
  const pretty = (c: string) => c.replace(':', ' · ').replace(':', ' · ').replace('allin', 'all-in');
  return <div className="calib">
    <StaleNote poll={dataPoll} />
    <p className="footnote">Every decision's predicted value is checked against what it actually won. Spots where the model is consistently off get a small, capped correction once there is enough evidence. {data ? `${data.active_corrections} active correction${data.active_corrections === 1 ? '' : 's'}.` : ''}</p>
    <div className="calib-list">{rows.slice(0, 14).map(r => <div key={r.category} className="calib-row">
      <span className="calib-cat">{pretty(r.category)}<small>{fmt(r.n)} decisions</small></span>
      <span className="calib-vals"><span>model {sgn(r.predicted_bb, 2)}</span><span className={r.realized_bb < r.predicted_bb ? 'negative' : 'positive'}>real {sgn(r.realized_bb, 2)}</span></span>
      <span className={`calib-bias ${r.bias_bb ? 'on' : ''}`} title={`residual ${sgn(r.residual_bb, 2)} ± ${fmt(r.se_bb, 2)} bb${r.bound_by ? ` · size set by ${r.bound_by}` : ''}${r.residual_pot != null ? ` · per pot ${sgn(r.residual_pot * 100, 1)}% ± ${fmt((r.se_pot || 0) * 100, 1)}% (candidate ${sgn((r.pot_bias_candidate || 0) * 100, 1)}%, not applied)` : ''}`}>{r.bias_bb ? `${sgn(r.bias_bb, 2)} bb` : r.n < 60 ? 'learning' : 'calibrated'}</span>
    </div>)}{!rows.length && <p className="footnote">Collecting the first outcomes…</p>}</div>
    {data && <LiveFits data={data} />}
  </div>;
}

interface LatencySummary { n: number; p50: number | null; p95: number | null; p99: number | null; max: number | null }
interface ComputeProfile { name: string; live_scale: number; learner_threads: number; analyst_threads: number }
interface ProfileState { active: ComputeProfile; stored: boolean; presets: ComputeProfile[]; logical_cores: number; min_live_scale: number }
interface ComputeState { decisions: LatencySummary; timeouts?: number; deadline_ms: number; max_share_of_deadline: number | null; live_samples: number; deal_chunks: number; logical_cores: number | null; load_average: number[] | null; profile?: ProfileState }

const PROFILE_HINT: Record<string, string> = { quiet: 'leave the PC free', balanced: 'about half the machine', max: 'every core (default)', custom: 'your own mix' };

/** Compute profile picker (0187): how much of this machine the fleet spends. Compute only; the
 * live budget never drops below the one the learner measured the policy at. */
function ProfilePicker({ state }: { state?: ProfileState }) {
  const [saved, setSaved] = useState<ProfileState>();
  const [custom, setCustom] = useState<ComputeProfile>();
  const [message, setMessage] = useState<{ text: string; error: boolean }>();
  const current = saved ?? state;
  if (!current) return null;
  const n = current.logical_cores;
  const draft = custom ?? { ...current.active, name: 'custom' };
  const save = async (body: object) => {
    setMessage(undefined);
    try {
      const r = await fetch('/api/compute/profile', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      const v = await r.json();
      if (!r.ok) throw new Error(v.detail || `Request failed (${r.status})`);
      setSaved(v); setCustom(undefined);
      setMessage({ text: `${v.active.name}: live bots now, learner and analyst restart with the new threads within a minute or so`, error: false });
    } catch (e) {
      setMessage({ text: e instanceof Error ? e.message : 'Could not save', error: true });
    }
  };
  const num = (k: keyof ComputeProfile, v: string) => setCustom({ ...draft, [k]: Number(v) });
  return <div className="profile-picker">
    <div className="profile-buttons" role="group" aria-label="Compute profile">
      {[...current.presets.map(p => p.name), 'custom'].map(name => <button key={name} type="button" className={current.active.name === name ? 'active' : ''} title={PROFILE_HINT[name]}
        onClick={() => name === 'custom' ? setCustom(draft) : save({ name })}>{name}</button>)}
    </div>
    {custom && <form className="profile-custom" onSubmit={e => { e.preventDefault(); save(draft); }}>
      <label>Live budget <input type="number" min={current.min_live_scale} max={1} step={0.05} value={draft.live_scale} onChange={e => num('live_scale', e.target.value)}/></label>
      <label>Learner threads <input type="number" min={1} max={n} step={1} value={draft.learner_threads} onChange={e => num('learner_threads', e.target.value)}/></label>
      <label>Analyst threads <input type="number" min={1} max={n} step={1} value={draft.analyst_threads} onChange={e => num('analyst_threads', e.target.value)}/></label>
      <button type="submit" className="button">Apply</button>
    </form>}
    <small>{PROFILE_HINT[current.active.name] ?? ''} · live ×{current.active.live_scale} · learner {current.active.learner_threads} · analyst {current.active.analyst_threads} of {n} threads</small>
    {message && <small className={message.error ? 'negative' : 'positive'} role="status">{message.text}</small>}
  </div>;
}

/** The season-13 #1 line: chips per hand the fleet must hold to finish first (0139). */
const TARGET_CHIPS_PER_HAND = 73;

/** Live pulse (0162): the questions the operator asks first. Are we on pace for #1, are the bots
 * playing, and is the decision path fast enough for the 45 s deadline? */
export function LivePulse({ bots }: { bots: Bot[] }) {
  const computePoll = usePoll<ComputeState>('/compute', 30000);
  const compute = computePoll.data;
  const playing = bots.filter(b => b.connected && b.status !== 'offline').length;
  const season = bots.reduce((acc, b) => ({ net: acc.net + (b.metrics?.net_chips || 0), hands: acc.hands + (b.metrics?.hands || 0) }), { net: 0, hands: 0 });
  const rate = season.hands ? season.net / season.hands : null;
  const tone = (r: number | null) => r == null ? '' : r >= TARGET_CHIPS_PER_HAND ? 'good' : r >= 50 ? 'warn' : 'bad';
  const d = compute?.decisions;
  const share = compute?.max_share_of_deadline;
  const scoped = bots[0]?.metrics?.season?.scoped;
  return <section className="live-pulse" aria-label="Live pulse">
    <StaleNote poll={computePoll} />
    <div className={`pulse-cell ${tone(rate)}`}>
      <span>{scoped ? `SEASON ${bots[0]?.metrics?.season?.number ?? ''} RATE` : 'RATE · ALL STORED HANDS'}</span>
      <b>{rate == null ? '—' : `${sgn(rate, 1)}`}<small> chips/hand</small></b>
      <small>target {TARGET_CHIPS_PER_HAND}{bots.map(b => <React.Fragment key={b.name}> · <PlayerName name={b.name}/> {b.metrics?.hands ? sgn(b.metrics.net_chips / b.metrics.hands, 0) : '—'}</React.Fragment>)}</small>
    </div>
    <div className={`pulse-cell ${playing === bots.length && bots.length ? 'good' : playing ? 'warn' : 'bad'}`}>
      <span>BOTS PLAYING</span>
      <b>{playing}<small> / {bots.length}</small></b>
      <small>{bots.filter(b => !b.connected || b.status === 'offline').map(b => b.name).join(', ') || 'every seat live'}</small>
    </div>
    <div className={`pulse-cell ${share == null ? '' : share < 0.05 ? 'good' : share < 0.2 ? 'warn' : 'bad'}`}>
      <span>DECISION TIME · LAST HOUR</span>
      <b>{d?.p95 == null ? '—' : `${fmt(d.p95, 0)}`}<small> ms p95</small></b>
      <small>{d?.n ? `p50 ${fmt(d.p50 || 0, 0)} · max ${fmt(d.max || 0, 0)} ms = ${((share || 0) * 100).toFixed(2)}% of the 45 s deadline · ${fmt(d.n)} decisions${compute?.timeouts ? ` · ${fmt(compute.timeouts)} took the safe action at the 8 s cap` : ''}` : 'no decisions this hour'}</small>
    </div>
    <div className="pulse-cell">
      <span>COMPUTE</span>
      <b>{compute ? fmt(compute.live_samples) : '—'}<small> samples</small></b>
      <small>{compute ? `${compute.deal_chunks} parallel chunks · load ${compute.load_average?.map(l => l.toFixed(1)).join(' / ') ?? '—'} on ${compute.logical_cores ?? '?'} threads · heads-up exact when it fits` : 'loading'}</small>
      <ProfilePicker state={compute?.profile}/>
    </div>
  </section>;
}



interface RivalCard { name: string; hands: number; bb_per_100: number; low_95: number; high_95: number; beats_us: boolean; we_beat: boolean; avatar_url?: string | null }
interface RivalsState { min_hands: number; opponents: number; nemeses: RivalCard[]; donors: RivalCard[] }

function RivalAvatar({ card }: { card: RivalCard }) {
  const [broken, setBroken] = useState(false);
  return <span className="rival-avatar">{card.avatar_url && !broken ? <img src={card.avatar_url} alt="" referrerPolicy="no-referrer" loading="lazy" onError={() => setBroken(true)}/> : card.name.slice(0, 2).toUpperCase()}</span>;
}

/** Rivals (0180): who takes our chips and who gives them, over every stored hand the champion
 * shared (150+; the experiment arms' treatment hands are out of the ledger, 0361), with the 95%
 * interval so a hot streak is not mistaken for a nemesis. */
export function RivalsPanel() {
  const dataPoll = usePoll<RivalsState>('/rivals', 60000);
  const data = dataPoll.data;
  if (!data) return <p className="subtle rivals-empty">{dataPoll.error ? `Unavailable: ${dataPoll.error}` : 'Loading rivals…'}</p>;
  const row = (c: RivalCard, kind: 'nemesis' | 'donor') => <li key={c.name} className={`rival ${kind} ${c.beats_us ? 'proven' : ''} ${c.we_beat ? 'proven' : ''}`}>
    <RivalAvatar card={c}/>
    <span className="rival-name"><PlayerName name={c.name}/>{c.beats_us && <em title="Beats us at 95%, corrected for every opponent tested">WANTED</em>}{c.we_beat && <em title="We beat them at 95%, corrected for every opponent tested">FAN CLUB</em>}</span>
    <b className={c.bb_per_100 < 0 ? 'negative' : 'positive'}>{sgn(c.bb_per_100, 0)}<small> bb/100</small></b>
    <small>{fmt(c.hands)} hands · 95% {sgn(c.low_95, 0)}..{sgn(c.high_95, 0)}</small>
  </li>;
  return <div className="rivals">
    <StaleNote poll={dataPoll} />
    <h3><Swords size={13}/> Nemeses <small>they take our chips</small></h3>
    {data.nemeses.length ? <ul>{data.nemeses.map(c => row(c, 'nemesis'))}</ul> : <p className="subtle">Nobody beats us over {data.min_hands}+ hands.</p>}
    <h3><Trophy size={13}/> Best customers <small>we take theirs</small></h3>
    {data.donors.length ? <ul>{data.donors.map(c => row(c, 'donor'))}</ul> : <p className="subtle">No regulars yet.</p>}
    <p className="subtle">{fmt(data.opponents)} opponents with {data.min_hands}+ shared hands of champion play (experiment-arm hands excluded).</p>
  </div>;
}


interface BadgeBot { name: string; score: number | null; hands_played: number | null; win_rate: number | null; rank_score: number | null; rank_win_rate: number | null; rank_hands_played: number | null; medal: 'gold' | 'silver' | 'bronze' | null; to_badge: number | null; to_prize: number | null }
interface BadgeState { bots: BadgeBot[]; bronze_score?: number | null; prize_line_score?: number | null; badge_ranks?: number; prize_ranks?: number; updated_at: number | null; error?: string | null }

const MEDAL: Record<string, string> = { gold: '🥇', silver: '🥈', bronze: '🥉' };

/** Badge race (0182): the season-end rewards each bot is chasing. Top 3 by score earn permanent
 * badges, the top 30 share the prize pool (one winning bot per owner). Ranks come from the server. */
export function BadgeRace() {
  const dataPoll = usePoll<BadgeState>('/badges', 60000);
  const data = dataPoll.data;
  if (!data) return <p className="subtle badge-empty">{dataPoll.error ? `Unavailable: ${dataPoll.error}` : 'Loading the badge race…'}</p>;
  const bots = [...data.bots].sort((a, b) => (a.rank_score ?? 1e9) - (b.rank_score ?? 1e9));
  return <div className="badge-race">
    <StaleNote poll={dataPoll} />
    {data.error && <p className="footnote amber">{data.updated_at ? 'Showing the last good read: ' : ''}{data.error}</p>}
    <ul>{bots.map(b => <li key={b.name} className={b.medal ? `medal-${b.medal}` : ''}>
      <span className="badge-medal" aria-label={b.medal ?? 'no medal'}>{b.medal ? MEDAL[b.medal] : b.rank_score && b.rank_score <= (data.prize_ranks ?? 30) ? '💰' : '·'}</span>
      <span className="badge-name"><PlayerName name={b.name}/><small>{b.rank_score ? `#${b.rank_score} score` : 'unranked'} · #{b.rank_win_rate ?? '—'} win rate · #{b.rank_hands_played ?? '—'} hands</small></span>
      <b>{b.to_badge === 0 ? 'ON A BADGE' : b.to_badge != null ? `+${fmt(b.to_badge)}` : '—'}<small>{b.to_badge ? 'to bronze' : b.to_badge === 0 ? 'hold it' : ''}</small></b>
    </li>)}</ul>
    <p className="subtle">Bronze line {data.bronze_score != null ? fmt(data.bronze_score) : '—'} · prize line (#{data.prize_ranks ?? 30}) {data.prize_line_score != null ? fmt(data.prize_line_score) : '—'} · win rate = share of hands won · prizes: one winning bot per owner</p>
  </div>;
}


interface StoryHand { bot: string; hand_id: string; ts: number; net: number; net_bb: number; pot_bb: number }
interface StoriesState { recap: { window_hours: number; hands: number; net: number; bots: { bot: string; hands: number; net: number }[]; best: StoryHand | null; worst: StoryHand | null }; timeline: { ts: number; kind: 'milestone' | 'big_pot'; text: string; bot?: string; hand_id?: string }[] }

/** A story's moment: the weekday with the time the viewer's clock reads (0295). */
const when = (ts: number) => time(ts, { weekday: true });

/** Stories (0178): the last 24 hours as a recap and the season as a timeline. */
export function StoriesPanel() {
  const dataPoll = usePoll<StoriesState>('/stories', 120000);
  const data = dataPoll.data;
  if (!data) return <p className="subtle stories-empty">{dataPoll.error ? `Unavailable: ${dataPoll.error}` : 'Loading stories…'}</p>;
  const r = data.recap;
  return <div className="stories">
    <StaleNote poll={dataPoll} />
    <section className="recap">
      <h3><Sparkles size={13}/> Today on the tables <small>last {r.window_hours} h · {fmt(r.hands)} hands</small></h3>
      <p className="recap-net">Fleet <b className={r.net < 0 ? 'negative' : 'positive'}>{sgn(r.net)}</b> chips</p>
      <ul className="recap-bots">{r.bots.map(b => <li key={b.bot}><span><PlayerName name={b.bot}/></span><b className={b.net < 0 ? 'negative' : 'positive'}>{sgn(b.net)}</b><small>{fmt(b.hands)} hands</small></li>)}</ul>
      <div className="recap-hands">
        {r.best && <p><Trophy size={12}/> Pot of the day: <b><PlayerName name={r.best.bot}/></b> <span className="positive">{sgn(r.best.net_bb, 0)} bb</span> in a {fmt(r.best.pot_bb)} bb pot at {when(r.best.ts)}</p>}
        {r.worst && <p><TrendingDown size={12}/> Cooler of the day: <b><PlayerName name={r.worst.bot}/></b> <span className="negative">{sgn(r.worst.net_bb, 0)} bb</span> at {when(r.worst.ts)}</p>}
      </div>
    </section>
    <section className="timeline">
      <h3><Crown size={13}/> This season</h3>
      <ol>{data.timeline.map((e, i) => <li key={i} className={e.kind}><time>{when(e.ts)}</time><span><NamesIn text={e.text}/></span></li>)}</ol>
    </section>
  </div>;
}
