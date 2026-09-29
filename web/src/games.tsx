import { useEffect, useState } from 'react';
import { StaleNote, usePoll } from './api';
import { PlayerName } from './playername';
import { SUITS, fmt } from './format';
import { readLocal, writeLocal } from './storage';
import type { AccuracyState, DecisionReport as Report, Grade, WiringState } from './types';

/** Decision grades (0220): the analyst's deep re-solves graded chess-style, and a quiz on real decisions. */
const GRADES: Grade[] = ['best', 'good', 'inaccuracy', 'mistake', 'blunder'];
const GRADE_LABEL: Record<Grade, string> = {best: 'Best', good: 'Good', inaccuracy: 'Inaccuracy', mistake: 'Mistake', blunder: 'Blunder'};
interface QuizOption { action: string; amount: number | null; ev_bb: number; loss_bb: number; grade: Grade; accuracy: number; fold_prob?: number; chosen_by_bot: boolean }
interface QuizSpot { id: number; bot: string; street: string; hole: string[]; board: string[]; pot: number; to_call: number; bb: number; opponents: number; bot_action: string; options: QuizOption[]; reason?: string }

const suit = SUITS;

function MiniCard({card}: {card: string}) {
  return <span className={`playing-card small ${'hd'.includes(card[1]) ? 'red' : ''}`} aria-label={card}><b>{card[0]}</b><span>{suit[card[1]]}</span></span>;
}

function GradeBar({report}: {report: Report}) {
  const total = Math.max(1, report.decisions);
  return <div className="gr-bar" role="img" aria-label={GRADES.map((g, i) => `${GRADE_LABEL[g]} ${report.grades[i]}`).join(', ')}>
    {GRADES.map((g, i) => report.grades[i] > 0 && <i key={g} className={`gr-${g}`} style={{width: `${report.grades[i] / total * 100}%`}}/>)}
  </div>;
}

export function AccuracyPanel() {
  const poll = usePoll<AccuracyState>('/accuracy', 120_000);
  const data = poll.data;
  if (!data) return <><StaleNote poll={poll}/>{!poll.error && <p className="subtle">Loading decision grades…</p>}</>;
  if (!data.fleet.decisions) return <><StaleNote poll={poll}/><p className="subtle">No big decision has been re-solved by the analyst in the last {data.days} days.</p></>;
  return <div className="gr-panel">
    <StaleNote poll={poll}/>
    <div className="gr-hero"><b>{fmt(data.fleet.accuracy, 1, true)}<small>%</small></b><span>ACCURACY · {fmt(data.fleet.decisions, 0, true)} decisions re-solved in {data.days} days · {fmt(data.fleet.mean_loss_bb, 3, true)} bb lost per decision</span></div>
    <GradeBar report={data.fleet}/>
    <div className="gr-legend">{GRADES.map((g, i) => <span key={g}><i className={`gr-${g}`}/>{GRADE_LABEL[g]} {fmt(data.fleet.grades[i], 0, true)}</span>)}</div>
    <table className="gr-table"><tbody>{data.bots.map(b => <tr key={b.bot}><td><PlayerName name={b.bot}/></td><td className="gr-acc">{fmt(b.report.accuracy, 1, true)}%</td><td><GradeBar report={b.report}/></td><td className="subtle">{fmt(b.report.grades[3] + b.report.grades[4], 0, true)} errors</td></tr>)}</tbody></table>
    {data.worst.length > 0 && <><h4 className="gr-h">BIGGEST ERRORS</h4><ul className="gr-worst">{data.worst.slice(0, 5).map((w, i) => <li key={i}><span className={`gr-chip gr-${w.grade}`}>{GRADE_LABEL[w.grade]}</span><b><PlayerName name={w.bot}/></b> {w.street}: {w.live_action.replace('_', ' ')} → deep {w.deep_action.replace('_', ' ')} <span className="subtle">−{fmt(w.loss_bb, 1, true)} bb of {fmt(w.pot_bb, 0, true)} bb</span></li>)}</ul></>}
    <p className="footnote">The analyst re-solves the bots' big decisions with far more samples; each live choice is graded by the EV it gave up against the deep search's best, as a share of the pot (Best under 2%, Good 10%, Inaccuracy 20%, Mistake 40%, Blunder beyond). Accuracy uses Lichess's formula.</p>
  </div>;
}

const age = (secs: number) => secs < 3600 ? `${Math.max(1, Math.round(secs / 60))} min` : secs < 172_800 ? `${Math.round(secs / 3600)} h` : `${Math.round(secs / 86_400)} days`;

/** The wiring table (0316): what each live component is worth, measured by re-running the newest
 * recorded big decisions with it switched off. A component that moves nothing is not a lever. */
export function WiringPanel() {
  const poll = usePoll<WiringState>('/wiring', 300_000);
  const data = poll.data;
  if (!data) return <><StaleNote poll={poll}/>{!poll.error && <p className="subtle">Loading the wiring table…</p>}</>;
  if (!data.available) return <><StaleNote poll={poll}/><p className="subtle">Unavailable: {data.reason}.</p></>;
  const r = data.report;
  const rows = [...r.rows].sort((a, b) => b.cost_bb - a.cost_bb || b.changed - a.changed);
  return <div className="gr-panel wiring">
    <StaleNote poll={poll}/>
    <p className={data.stale ? 'footnote amber' : 'footnote'}>Measured {age(data.age_secs)} ago on the newest {fmt(r.sample, 0, true)} recorded big decisions{r.streets?.length ? ` (${r.streets.map(([s, n]) => `${s} ${fmt(n, 0, true)}`).join(', ')})` : ''}{data.stale ? ' — older than two days: the analyst has not re-measured it' : ''}.</p>
    <table className="gr-table wiring-table">
      <thead><tr><th>COMPONENT SWITCHED OFF</th><th>MOVES</th><th title="EV the moved choice gives up under the full model, per decision in the sample">BB / DEC</th><th title="Largest single cost in the sample">MAX BB</th></tr></thead>
      <tbody>{rows.map(row => <tr key={row.component} className={row.changed ? '' : 'wiring-idle'}>
        <td>{row.component}</td>
        <td className="gr-acc" title={row.installed == null ? undefined : `on ${row.installed} of ${r.sample} spots`}>{row.changed ? `${fmt(row.share_pct, row.share_pct < 10 ? 1 : 0, true)}%` : row.installed === 0 ? 'not measured' : 'none'}</td>
        <td className="gr-acc">{fmt(row.cost_bb, 2, true)}</td>
        <td className="gr-acc">{row.max_bb ? fmt(row.max_bb, 1, true) : '—'}</td>
      </tr>)}</tbody>
    </table>
    {!!r.calibration?.length && <>
      <h4 className="gr-h">SELF-CALIBRATION · EVERY DECISION, LAST 24 H</h4>
      <table className="gr-table wiring-table wiring-calibration">
        <thead><tr><th>STREET</th><th>DECISIONS</th><th title="Best action changes when the calibration bias on each option is removed">DECIDED BY IT</th><th>MOSTLY</th></tr></thead>
        <tbody>{r.calibration.map(f => <tr key={f.street} className={f.flipped ? '' : 'wiring-idle'}>
          <td>{f.street}</td>
          <td className="gr-acc">{fmt(f.decisions, 0, true)}</td>
          <td className="gr-acc">{fmt(f.share_pct, f.share_pct < 10 ? 1 : 0, true)}%</td>
          <td className="gr-acc">{f.main_count ? f.main : '—'}</td>
        </tr>)}</tbody>
      </table>
    </>}
    <p className="footnote">{r.v3 ? `${fmt(r.v3_exact, 0, true)} of ${fmt(r.v3, 0, true)} records that carry the live inputs replay exactly as played` : 'No record in the sample carries the live inputs yet (replay v3)'} · {fmt(r.unstable, 0, true)} gave different answers on identical inputs · {fmt(r.chosen_not_best, 0, true)} chose below the best EV (mixing).</p>
    <p className="footnote">“Not measured”: no spot in the sample had the component to switch off (a preflop fit on big spots that are mostly postflop, say), which is not the same as moving none. “Moves”: decisions whose action or size changes with the component off. “bb / dec”: what that change gives up under the full model, same cards and samples — the component's value on these spots. The analyst re-measures daily when its audit queue is empty; <code>review wiring</code> runs it by hand.</p>
  </div>;
}

interface Score { answered: number; accuracySum: number; streak: number; best: number }
const SCORE_KEY = 'svan-quiz-score:v1';
function loadScore(): Score {
  // The read itself cannot throw (storage.ts); the parse still can on a value that is not JSON.
  try { return {answered: 0, accuracySum: 0, streak: 0, best: 0, ...JSON.parse(readLocal(SCORE_KEY) || '{}')}; } catch { return {answered: 0, accuracySum: 0, streak: 0, best: 0}; }
}
function saveScore(s: Score) { writeLocal(SCORE_KEY, JSON.stringify(s)); }
const label = (o: QuizOption, bb: number) => o.action === 'raise' && o.amount != null ? `Raise to ${fmt(o.amount / bb, 1, true)} bb` : o.action === 'all_in' ? 'All in' : o.action[0].toUpperCase() + o.action.slice(1);

export function QuizPage() {
  const [spot, setSpot] = useState<QuizSpot>();
  const [error, setError] = useState('');
  const [picked, setPicked] = useState<number>();
  const [score, setScore] = useState<Score>(loadScore);
  const next = () => {
    setPicked(undefined); setError('');
    fetch('/api/quiz').then(async r => { if (!r.ok) throw new Error((await r.json().catch(() => ({}))).detail || `Request failed (${r.status})`); return r.json(); })
      .then(setSpot).catch(e => setError((e as Error).message));
  };
  useEffect(next, []);
  const answer = (i: number) => {
    if (!spot || picked != null) return;
    setPicked(i);
    const o = spot.options[i];
    const good = o.grade === 'best' || o.grade === 'good';
    const s = {answered: score.answered + 1, accuracySum: score.accuracySum + o.accuracy, streak: good ? score.streak + 1 : 0, best: Math.max(score.best, good ? score.streak + 1 : 0)};
    setScore(s); saveScore(s);
  };
  const bestEv = spot ? Math.max(...spot.options.map(o => o.ev_bb)) : 0;
  const worstEv = spot ? Math.min(...spot.options.map(o => o.ev_bb)) : 0;
  return <div className="quiz-page">
    <header className="quiz-head"><div><span className="pc-kicker">TRAINING GROUND</span><h1>What would Svanbot do?</h1><p className="subtle">A real decision from our tables. Pick the play; it is graded by the EV it gives up against the bot's best option.</p></div>
      <div className="quiz-score"><div><span>ACCURACY</span><b>{score.answered ? fmt(score.accuracySum / score.answered, 1, true) : '—'}</b></div><div><span>STREAK</span><b>{score.streak}</b></div><div><span>BEST</span><b>{score.best}</b></div><div><span>PLAYED</span><b>{score.answered}</b></div></div></header>
    {error && <p className="pc-error">{error}</p>}
    {spot && <section className="quiz-spot" aria-label="Quiz spot">
      <div className="quiz-situation">
        <div><span>YOUR HAND</span><div className="quiz-cards">{(spot.hole || []).map(c => <MiniCard key={c} card={c}/>)}</div></div>
        <div><span>BOARD · {spot.street.toUpperCase()}</span><div className="quiz-cards">{(spot.board || []).length ? spot.board.map(c => <MiniCard key={c} card={c}/>) : <em className="subtle">preflop</em>}</div></div>
        <div><span>POT</span><b>{fmt(spot.pot / spot.bb, 1, true)} bb</b></div>
        <div><span>TO CALL</span><b>{spot.to_call ? `${fmt(spot.to_call / spot.bb, 1, true)} bb` : '—'}</b></div>
        <div><span>OPPONENTS</span><b>{spot.opponents}</b></div>
      </div>
      <div className="quiz-options">{spot.options.map((o, i) => {
        const shown = picked != null;
        return <button key={i} className={`quiz-option ${shown ? `gr-${o.grade}-bg` : ''} ${picked === i ? 'picked' : ''}`} disabled={shown} onClick={() => answer(i)}>
          <b>{label(o, spot.bb)}</b>
          {shown && <><span className="quiz-ev"><i style={{width: `${bestEv === worstEv ? 100 : (o.ev_bb - worstEv) / (bestEv - worstEv) * 100}%`}}/></span><small>EV {o.ev_bb >= 0 ? '+' : ''}{fmt(o.ev_bb, 1, true)} bb · {GRADE_LABEL[o.grade]}{o.chosen_by_bot ? ' · Svanbot chose this' : ''}</small></>}
        </button>;
      })}</div>
      {picked != null && <div className={`quiz-verdict gr-${spot.options[picked].grade}-bg`}><b>{GRADE_LABEL[spot.options[picked].grade]}</b><span>{spot.options[picked].loss_bb < 0.05 ? 'You found the best play.' : `You gave up ${fmt(spot.options[picked].loss_bb, 1, true)} bb against the best option.`} Svanbot (<PlayerName name={spot.bot}/>) played {spot.bot_action.replace('_', ' ')}.</span>
        <button className="button primary" onClick={next}>Next spot</button></div>}
    </section>}
  </div>;
}
