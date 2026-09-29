import React, { useEffect, useState } from 'react';
import { request, StaleNote, usePoll } from './api';
import { fmt as fmtNum, sgn as sgnNum } from './format';
import { PlayerName } from './playername';

const fmt = (v: number | null | undefined, d = 0) => fmtNum(v, d, true);
const sgn = (v: number | null | undefined, d = 0) => sgnNum(v, d, true);
const pct = (v: number | null | undefined, d = 0) => v == null ? '—' : `${fmt(v * 100, d)}%`;
const RANKS = 'AKQJT98765432';

interface LineRow { key: string; label: string; hands: number; total_chips: number; bb_per_hand: number; low_bb: number; high_bb: number; share_bb100: number }
interface Suggestion { severity: 'high' | 'medium' | 'info'; title: string; evidence: string; action: string }
interface Trip { street: string; bet_into_us: number; bet_into_us_n: number; bet_elsewhere: number; bet_elsewhere_n: number; z: number; our_fold: number; faced: number; mdf_fold: number; flagged: boolean }
interface Result { hands?: number; bb100: number; low_bb100: number; high_bb100: number; chips: number }
interface Analysis {
  hands: number;
  /** This season's result, and which season that is. */
  season?: Result & { scoped: boolean; number: number | null; started_at: number | null };
  /** Every recorded hand: the leaks themselves are learning and carry across seasons. */
  overall: Result;
  costly_lines: LineRow[]; best_lines: LineRow[]; outcomes: LineRow[]; positions: LineRow[];
  trend: { block: number; hands_end: number; bb100: number; cumulative_bb: number }[];
  tripwires: Trip[];
  opponents: { name: string; hands: number; bb100: number; upper_bb100: number; beats_us: boolean }[];
  suggestions: Suggestion[];
}

/** Interval bar: the 95% range of bb per hand around zero, with the mean marked. */
function Interval({ row, scale }: { row: LineRow; scale: number }) {
  const x = (v: number) => 50 + Math.max(-50, Math.min(50, v / scale * 50));
  const lo = x(isFinite(row.low_bb) ? row.low_bb : -scale), hi = x(isFinite(row.high_bb) ? row.high_bb : scale);
  return <span className="interval" title={`95%: ${sgn(row.low_bb, 2)} .. ${sgn(row.high_bb, 2)} bb per hand`}>
    <i className="interval-zero" />
    <i className={`interval-range ${row.high_bb < 0 ? 'neg' : row.low_bb > 0 ? 'pos' : ''}`} style={{ left: `${lo}%`, width: `${Math.max(1, hi - lo)}%` }} />
    <i className="interval-mean" style={{ left: `${x(row.bb_per_hand)}%` }} />
  </span>;
}

function LineTable({ rows, empty }: { rows: LineRow[]; empty: string }) {
  const scale = Math.max(1, ...rows.map(r => Math.abs(r.bb_per_hand) * 2));
  if (!rows.length) return <p className="footnote">{empty}</p>;
  return <div className="data-table-wrap"><table className="data-table lab-table"><thead><tr><th>SPOT</th><th>HANDS</th><th>CHIPS</th><th>BB / HAND · 95%</th><th>BB/100 SHARE</th></tr></thead>
    <tbody>{rows.map(r => <tr key={r.key}><td title={r.key}><b>{r.label || r.key}</b><small>{r.key}</small></td><td>{fmt(r.hands)}</td><td className={r.total_chips < 0 ? 'negative' : 'positive'}>{sgn(r.total_chips)}</td><td><Interval row={r} scale={scale} /><small>{sgn(r.bb_per_hand, 2)}</small></td><td className={r.share_bb100 < 0 ? 'negative' : 'positive'}>{sgn(r.share_bb100, 1)}</td></tr>)}</tbody></table></div>;
}

function Trend({ points }: { points: Analysis['trend'] }) {
  if (points.length < 2) return <p className="footnote">The trend needs at least 500 hands.</p>;
  const w = 400, h = 150, pad = 12;
  const cum = points.map(p => p.cumulative_bb);
  const lo = Math.min(0, ...cum), hi = Math.max(1, ...cum);
  const X = (i: number) => pad + i / (points.length - 1) * (w - 2 * pad);
  const Y = (v: number) => h - pad - (v - lo) / (hi - lo) * (h - 2 * pad);
  const maxBar = Math.max(1, ...points.map(p => Math.abs(p.bb100)));
  return <div className="lab-trend">
    <svg viewBox={`0 0 ${w} ${h}`} role="img" aria-label="Cumulative big blinds won">
      <line x1={pad} x2={w - pad} y1={Y(0)} y2={Y(0)} stroke="var(--line)" strokeDasharray="3 5" />
      {points.map((p, i) => <rect key={i} x={X(i) - 3} width={6} y={p.bb100 >= 0 ? Y(0) - p.bb100 / maxBar * 30 : Y(0)} height={Math.abs(p.bb100) / maxBar * 30} fill={p.bb100 >= 0 ? '#6daa9855' : '#b6857855'}><title>{`Hands ${p.hands_end - 250}–${p.hands_end}: ${sgn(p.bb100, 0)} bb/100`}</title></rect>)}
      <polyline points={points.map((p, i) => `${X(i)},${Y(p.cumulative_bb)}`).join(' ')} fill="none" stroke="#e4b956" strokeWidth="2.2" />
    </svg>
    <p className="footnote">Line: cumulative big blinds. Bars: bb/100 of each 250-hand block (hover for values). Blocks swing widely; trust the line.</p>
  </div>;
}

export function LeakFinder() {
  const analysis = usePoll<Analysis>('/analysis', 300000);
  const { data, refresh } = analysis;
  const [tab, setTab] = useState<'advice' | 'lines' | 'outcomes' | 'positions' | 'opponents' | 'exploit' | 'trend'>('advice');
  const tabs: [typeof tab, string][] = [['advice', 'Advice'], ['lines', 'Lines'], ['outcomes', 'Endings'], ['positions', 'Positions'], ['opponents', 'Opponents'], ['exploit', 'Exploit check'], ['trend', 'Trend']];
  if (!data) return <p className="footnote">{analysis.error ? `Analysis unavailable: ${analysis.error}` : 'Analysing every recorded hand…'}</p>;
  // A server without the season split (older build mid-release) reports only the lifetime result.
  const result = data.season ?? { ...data.overall, scoped: false, number: null, started_at: null };
  const season = result.scoped ? `SEASON ${result.number ?? '—'}` : 'ALL STORED HANDS';
  return <div className="lab">
    <StaleNote poll={analysis} />
    <div className="lab-summary">
      <div><label>HANDS ANALYSED</label><b>{fmt(data.hands)}</b><small>every season</small></div>
      <div><label>WIN RATE · {season}</label><b className={result.bb100 < 0 ? 'negative' : 'positive'}>{sgn(result.bb100, 1)}</b><small>bb/100 · 95% {sgn(result.low_bb100, 0)} .. {sgn(result.high_bb100, 0)} · {fmt(result.hands ?? data.hands)} hands</small></div>
      <div><label>NET · {season}</label><b className={result.chips < 0 ? 'negative' : 'positive'}>{sgn(result.chips)}</b><small>chips · lifetime {sgn(data.overall.chips)} at {sgn(data.overall.bb100, 1)} bb/100</small></div>
      <button className="button" onClick={refresh} title="Reports refresh every five minutes">Refresh</button>
    </div>
    <p className="footnote">The leaks below read every recorded hand: the strategy carries across seasons, so a leak does too. Only the headline result above is this season's.</p>
    <div className="position-tabs lab-tabs">{tabs.map(([k, label]) => <button key={k} className={tab === k ? 'active' : ''} onClick={() => setTab(k)}>{label}</button>)}</div>
    {tab === 'advice' && <div className="lab-advice">{data.suggestions.map((s, i) => <div key={i} className={`advice ${s.severity}`}><span className="advice-sev">{s.severity.toUpperCase()}</span><b>{s.title}</b><p className="advice-evidence">{s.evidence}</p><p>{s.action}</p></div>)}{!data.suggestions.length && <p className="footnote">No leaks stand out yet.</p>}</div>}
    {tab === 'lines' && <><p className="footnote">Our own action sequence per street (B bet, R raise, C call, X check, F fold; - no action), worst first. 20+ hands each. The interval bar is red when a line loses with 95% confidence.</p><LineTable rows={data.costly_lines} empty="No line has 20 hands yet." /><p className="footnote">Best lines</p><LineTable rows={data.best_lines} empty="" /></>}
    {tab === 'outcomes' && <><p className="footnote">How hands ended for us. "Bet or raised, then folded" is the classic leak: chips put in with a hand that could not continue.</p><LineTable rows={data.outcomes} empty="Collecting hands…" /></>}
    {tab === 'positions' && <><p className="footnote">Results by seat. Blinds lose for everyone; compare the others against each other.</p><LineTable rows={data.positions} empty="Collecting hands…" /></>}
    {tab === 'opponents' && <div className="data-table-wrap"><table className="data-table lab-table"><thead><tr><th>OPPONENT</th><th>HANDS</th><th>OUR BB/100</th><th>95% UPPER</th><th /></tr></thead><tbody>{data.opponents.map(o => <tr key={o.name}><td><b><PlayerName name={o.name}/></b></td><td>{fmt(o.hands)}</td><td className={o.bb100 < 0 ? 'negative' : 'positive'}>{sgn(o.bb100, 0)}</td><td>{sgn(o.upper_bb100, 0)}</td><td>{o.beats_us ? <span className="tag amber">BEATS US</span> : ''}</td></tr>)}</tbody></table><p className="footnote">Chips that moved between us and each opponent in the champion hands they were dealt into (the experiment arms' treatment hands are excluded from the ledger, 0361): what they won from us, minus what we won from them. The 95% upper bound is theirs alone; "Beats us" needs 300+ hands and a bound below zero after correcting for every opponent tested, since among a hundred players a few clear 95% by luck.</p></div>}
    {tab === 'exploit' && <div className="data-table-wrap"><table className="data-table lab-table"><thead><tr><th>STREET</th><th>THEY BET INTO US</th><th>ELSEWHERE</th><th>Z</th><th>WE FOLD</th><th>MDF FOLD</th><th /></tr></thead><tbody>{data.tripwires.map(t => <tr key={t.street}><td><b>{t.street}</b></td><td>{pct(t.bet_into_us)}<small>{fmt(t.bet_into_us_n)} spots</small></td><td>{pct(t.bet_elsewhere)}<small>{fmt(t.bet_elsewhere_n)} spots</small></td><td className={t.z > 2 ? 'negative' : ''}>{sgn(t.z, 1)}</td><td>{pct(t.our_fold)}<small>{fmt(t.faced)} faced</small></td><td>{pct(t.mdf_fold)}</td><td>{t.flagged ? <span className="tag amber">TARGETED</span> : <span className="tag">OK</span>}</td></tr>)}</tbody></table><p className="footnote">Are opponents exploiting our folds? If they bet into us significantly more often than into others (z &gt; 2) while we fold more than the minimum defence frequency, their bets contain extra bluffs and we should call wider.</p></div>}
    {tab === 'trend' && <Trend points={data.trend} />}
  </div>;
}

interface RangeOpp { seat: number; name: string; position: string; grid: number[]; share: number[]; top: { hand: string; share: number }[]; equity: number | null; equity_se?: number | null; profile: { hands: number; vpip: number; pfr: number; confidence: number } }
interface Ranges { available: boolean; hole?: string[]; board?: string[]; street?: string; pot?: number; to_call?: number; position?: string; equity_vs_all?: number | null; equity_vs_all_se?: number | null; samples?: number; opponents?: RangeOpp[]; range_model?: string }

/** 13x13 heatmap: pairs on the diagonal, suited above, offsuit below. `names` is the engine's own
 * class order from `/api/hand-classes`: it places every cell, so a label it does not name is called
 * out rather than drawn as a measured zero. */
function RangeGrid({ grid, share, names }: { grid: number[]; share: number[]; names: string[] }) {
  const max = Math.max(...grid, 1e-9);
  const cells: React.ReactNode[] = [];
  for (let r = 0; r < 13; r++) for (let c = 0; c < 13; c++) {
    const hi = RANKS[Math.min(r, c)], lo = RANKS[Math.max(r, c)];
    const label = r === c ? hi + lo : c > r ? `${hi}${lo}s` : `${hi}${lo}o`;
    const idx = names.indexOf(label);
    const v = idx >= 0 ? grid[idx] / max : 0;
    const title = idx < 0
      ? `${label}: the engine's class order does not name this hand`
      : `${label}: likelihood ${pct(v, 0)} of the most likely hand · ${pct(share[idx], 1)} of the range`;
    cells.push(<span key={label} title={title} style={{ background: `rgba(228,185,86,${0.08 + v * 0.85})`, color: v > 0.45 ? '#1b1c14' : '#b2b9ac' }}>{label}</span>);
  }
  return <div className="range-grid heat">{cells}</div>;
}

/** The engine's class order, fetched once from `/hand-classes` and shared by every mount. A failed
 * fetch is deliberately not cached, so the next mount — or the grid's own retry — asks again. */
let CLASS_NAMES: string[] | null = null;

export function RangeExplorer({ slot, decisionKey }: { slot?: number; decisionKey?: string }) {
  const [names, setNames] = useState<string[] | null>(CLASS_NAMES);
  const [namesError, setNamesError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  // Without the engine's class order there is no grid to draw, so a failure is said out loud where
  // the grid goes and offers a retry (LESSONS 24: a panel that has no live source must not read as
  // a quiet empty one).
  useEffect(() => {
    if (CLASS_NAMES) return;
    let alive = true;
    setNamesError(null);
    request<string[]>('/hand-classes')
      .then((v: string[]) => { if (alive) { CLASS_NAMES = v; setNames(v); } })
      .catch((e: unknown) => { if (alive) setNamesError(e instanceof Error ? e.message : String(e)); });
    return () => { alive = false; };
  }, [attempt]);
  const ranges = usePoll<Ranges>(slot == null ? null : `/bots/${slot}/ranges`, 20000, decisionKey);
  const { data } = ranges;
  const [pick, setPick] = useState(0);
  if (!data?.available || !data.opponents?.length) return <><StaleNote poll={ranges}/>{!ranges.error && <p className="footnote">Ranges appear after the bot's next decision against live opponents.</p>}</>;
  const opp = data.opponents[Math.min(pick, data.opponents.length - 1)];
  const grid = names
    ? <RangeGrid grid={opp.grid} share={opp.share} names={names}/>
    : namesError
      ? <div className="error-banner" role="alert"><span>Range grid unavailable: the engine's hand classes did not load ({namesError}).</span><button className="button" onClick={() => setAttempt(n => n + 1)}>Retry</button></div>
      : <p className="footnote" role="status">Loading the engine's hand classes…</p>;
  return <div className="lab">
    <StaleNote poll={ranges} />
    <div className="lab-summary">
      <div><label>OUR HAND</label><b>{data.hole?.join(' ')}</b><small>{data.position} · {data.street}{data.board?.length ? ` · ${data.board.join(' ')}` : ''}</small></div>
      <div><label>EQUITY VS ALL</label><b>{pct(data.equity_vs_all, 1)}</b><small>{data.equity_vs_all_se != null ? `± ${(data.equity_vs_all_se * 196).toFixed(2)} pp · ` : ''}pot {fmt(data.pot)} · to call {fmt(data.to_call)}{data.samples ? ` · ${fmt(data.samples)} samples` : ''}</small></div>
      <div><label>RANGE MODEL</label><b className="small-b">{data.range_model}</b></div>
    </div>
    <div className="position-tabs lab-tabs">{data.opponents.map((o, i) => <button key={o.seat} className={i === pick ? 'active' : ''} onClick={() => setPick(i)}>{o.name.slice(0, 10)} · {o.position}</button>)}</div>
    {grid}
    <div className="lab-range-foot">
      <span>Our equity vs {opp.name}: <b>{pct(opp.equity, 1)}</b>{opp.equity_se != null ? <small> ± {(opp.equity_se * 196).toFixed(2)} pp</small> : null}</span>
      <span>Profile: {fmt(opp.profile.hands)} hands · VPIP {pct(opp.profile.vpip)} · PFR {pct(opp.profile.pfr)} · confidence {pct(opp.profile.confidence)}</span>
      <span>Most likely: {opp.top.map(t => `${t.hand} ${pct(t.share, 0)}`).join(' · ')}</span>
    </div>
    <p className="footnote">The estimated range of each opponent at the moment of our last decision, replayed from their actions and learned profile. Brighter = more likely per combo (hover for the share of the whole range). Equity is Monte Carlo against that range.</p>
  </div>;
}
