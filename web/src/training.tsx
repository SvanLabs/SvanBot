/** Season, performance and learner panels: autonomy controls, cooldown, experiments, compute profile. */
import React, { useState } from 'react';
import { Cpu, FlaskConical, History, Layers, Play, ShieldCheck, Square } from 'lucide-react';
import type { Bot, ChampionKnob, ClassRow, Experiment, ExperimentMode, Finding, Metrics, SearchFunnel, SeasonScope, Training } from './types';
import { usePoll, StaleNote } from './api';
import { format, signed, percent, ago, api, Panel } from './ui';
import { fmt, sgn, time } from './format';
import { PlayerName } from './playername';

export function scopeLabel(scope?: SeasonScope) {
  // No scope at all is a server without the season split: its figures are every stored hand.
  if (!scope) return 'All stored hands';
  return scope.scoped ? `Season ${scope.number ?? '—'}` : 'All stored hands · season unknown';
}

/** When a season-scoped panel starts, e.g. "since 20 Sep". */
export function seasonSince(scope?: SeasonScope) {
  return scope?.scoped && scope.started_at ? `since ${time(scope.started_at, { dateOnly: true })}` : '';
}

export function Performance({metrics}: {metrics?:Metrics}) {
  const series = metrics?.series || [];
  const scoped = !!metrics?.season?.scoped;
  const values = series.flatMap(point => [point.total, point.showdown, point.other, point.ev ?? point.total]);
  const hasEv = series.some(point => point.ev != null);
  const lower = Math.min(0, ...values), upper = Math.max(1, ...values);
  const y = (value:number) => 145 - (value - lower) / (upper - lower) * 126;
  const points = (key:'total'|'showdown'|'other'|'ev') => series.map((point,index) => `${15 + index / Math.max(1,series.length-1)*270},${y(point[key] ?? point.total)}`).join(' ');
  return <><div className="pnl-summary"><span>RECORDED NET WINNINGS · {scopeLabel(metrics?.season).toUpperCase()}</span><strong className={(metrics?.net_chips || 0) < 0 ? 'negative' : 'positive'}>{signed(metrics?.net_chips)}<small>chips</small></strong><span className="subtle">Recorded hands only · excludes buy-ins and rebuys{metrics?.all_time ? ` · lifetime ${signed(metrics.all_time.net_chips)}` : ''}</span></div>
    {metrics && !scoped && <p className="footnote amber">Not scoped to a season yet: {metrics.season ? 'the season start is unknown' : 'the running server predates season scoping (next release)'}, so these figures cover every stored hand.</p>}
    <div className="chart"><svg viewBox="0 0 300 170" role="img" aria-label={`Cumulative winnings by hand · ${scopeLabel(metrics?.season)}`}>{[20,62,104,146].map(height => <line key={height} x1="12" x2="288" y1={height} y2={height} stroke="var(--line)" strokeDasharray="3 5"/>)}{series.length > 0 && <>{(['total','showdown','other'] as const).map((key,index) => <polyline key={key} points={points(key)} fill="none" stroke={['#e4b956','#6daa98','#b68578'][index]} strokeWidth={index === 0 ? 2.5 : 1.5}/>)}{hasEv && <polyline className="ev-line" points={points('ev')} fill="none" stroke="#8fb3e8" strokeWidth={1.5} strokeDasharray="4 3"/>}</>}</svg>{series.length === 0 && <div className="chart-empty">Your performance story starts<br/>with the first completed hand.</div>}</div>
    <div className="chart-legend"><span><i className="legend-total"/>Total</span><span><i className="legend-showdown"/>Showdown</span><span><i className="legend-other"/>Non-showdown</span>{hasEv && <span title="Winnings with the luck of all-in runouts removed: each all-in before the river counts as its equity share of the pot (0213)"><i className="legend-ev"/>All-in EV</span>}</div>
    <div className="two-stats"><div><label>WIN RATE</label><b className={(metrics?.bb100 || 0) < 0 ? 'negative' : 'positive'}>{signed(metrics?.bb100,1)}<small> bb/100</small></b></div><div><label>95% INTERVAL</label><b>{metrics?.confidence == null ? '—' : `± ${format(metrics.confidence,1)}`}</b></div></div>
    {metrics?.ev_bb100 != null && <div className="two-stats ev-stats"><div><label>ALL-IN EV RATE</label><b className={metrics.ev_bb100 < 0 ? 'negative' : 'positive'}>{signed(metrics.ev_bb100,1)}<small> bb/100 ± {format(metrics.ev_confidence,1)}</small></b></div><div><label>ALL-IN LUCK</label><b className={(metrics.luck_chips || 0) < 0 ? 'negative' : 'positive'}>{signed(metrics.luck_chips)}<small> chips</small></b></div></div>}<p className="footnote">{format(metrics?.priced_hands || 0)} hands with verified stack accounting · {scopeLabel(metrics?.season).toLowerCase()}{seasonSince(metrics?.season) ? ` ${seasonSince(metrics?.season)}` : ''}{metrics?.all_time ? ` · ${format(metrics.all_time.hands)} lifetime` : ''}</p></>;
}

export function CooldownSetting({training}: {training?:Training}) {
  const cfg = training?.settings;
  const [draft,setDraft] = useState<{minutes?:string;hands?:string}>({});
  const [state,setState] = useState<{saving:boolean;message:string;error:boolean}>({saving:false,message:'',error:false});
  const minutesValue = draft.minutes ?? (cfg?.cooldown_minutes == null ? '' : String(cfg.cooldown_minutes));
  const handsValue = draft.hands ?? (cfg?.min_new_hands == null ? '' : String(cfg.min_new_hands));
  const maxMinutes = cfg?.max_cooldown_minutes ?? 1440;
  const maxHands = cfg?.max_min_new_hands ?? 20000;
  const minutes = Number(minutesValue), hands = Number(handsValue);
  const minutesOk = minutesValue.trim() !== '' && Number.isFinite(minutes) && minutes >= 0 && minutes <= maxMinutes;
  const handsOk = handsValue.trim() !== '' && Number.isInteger(hands) && hands >= 0 && hands <= maxHands;
  const changed = draft.minutes !== undefined || draft.hands !== undefined;
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!minutesOk || !handsOk) return;
    setState({saving:true,message:'',error:false});
    try {
      const body: Record<string, number> = {};
      if (draft.minutes !== undefined) body.cooldown_minutes = minutes;
      if (draft.hands !== undefined) body.min_new_hands = hands;
      const r = await api<{cooldown_minutes:number;min_new_hands:number}>('/training/settings', body);
      setDraft({});
      setState({saving:false,message:`Saved: champion search after ${format(r.min_new_hands)} new hands after season day 3; ${format(r.cooldown_minutes)} min search cooldown; evidence refreshes run on the hands as they arrive`,error:false});
    } catch (e) {
      setState({saving:false,message:e instanceof Error ? e.message : 'Could not save',error:true});
    }
  };
  const cooling = training?.phase === 'cooling down' && training.cooldown_until && training.cooldown_until > Date.now()/1000;
  return <form className="cooldown-setting" onSubmit={save} aria-label="Learner pacing">
    <label htmlFor="learner-hands"><span className="eyebrow">NEW HANDS BEFORE A CHAMPION SEARCH</span><small>Caps the champion-search wait after season day 3. During days 1–3 searches run after cooldown. Evidence refreshes are not set here: they run on the hands as they arrive. The promotion gate is unchanged.</small></label>
    <div><input id="learner-hands" type="number" min={0} max={maxHands} step={50} inputMode="numeric" value={handsValue} onChange={e => setDraft(d => ({...d, hands: e.target.value}))} aria-invalid={!handsOk}/><span>hands</span></div>
    <label htmlFor="learner-cooldown"><span className="eyebrow">COOLDOWN BETWEEN SEARCHES</span><small>{cooling ? `Cooling down until ${time(training!.cooldown_until!, { seconds: true })}. ` : ''}Only searches wait on the clock; evidence refreshes run on the hands as they arrive.</small></label>
    <div><input id="learner-cooldown" type="number" min={0} max={maxMinutes} step={5} inputMode="numeric" value={minutesValue} onChange={e => setDraft(d => ({...d, minutes: e.target.value}))} aria-invalid={!minutesOk}/><span>min</span><button className="button" type="submit" disabled={!minutesOk || !handsOk || state.saving || !changed}>{state.saving ? 'Saving…' : 'Save'}</button></div>
    {state.message && <p className={`footnote ${state.error ? 'negative' : 'positive'}`} role="status">{state.message}</p>}
  </form>;
}

/** The explanation a family of findings shares (#321). The server states it once per family, keyed by
 *  an id prefix (`decision`, `calibration`, `style-drift`), so forty rows do not repeat one paragraph;
 *  the longest key that prefixes a finding's id is the family it belongs to. */
function legendFor(legends: Record<string,string>, id: string): string {
  let best = '';
  for (const prefix of Object.keys(legends)) if (id.startsWith(prefix) && prefix.length > best.length) best = prefix;
  return best ? legends[best] : '';
}

/** What the class table's state column says, in the reader's terms (#321). `thin` is the class the old
 *  panel printed as a finding, above the line saying nothing could be decided. */
const CLASS_STATE: Record<string,string> = { filed: 'P0 · filed', measured: 'measured', thin: 'not yet decidable', 'never queued': 'never queued' };

/** The deep re-solve's classes as a table (#321): class, n, bb per decision, its interval and the
 *  decisions in the window — the columns every row used to spell out in its own paragraph. */
function ClassTable({classes, legend, findings}: {classes:ClassRow[]; legend:string; findings:Finding[]}) {
  return <div className="class-table-wrap">
    {legend && <p className="pc-note family-legend">{legend}</p>}
    <table className="pc-table class-table">
      <caption className="pc-note">Every class the deep re-solve measured, the losses first.</caption>
      <thead><tr><th>Class</th><th>n</th><th>bb / decision</th><th>95%</th><th>Decisions in window</th><th/></tr></thead>
      <tbody>{classes.map(c => {
        const filed = findings.find(f => f.id === c.id);
        return <tr key={c.id} className={`class-${c.state.replace(' ', '-')}`}>
          <td>{c.label}</td>
          <td>{format(c.n)}</td>
          <td className={c.state === 'filed' ? 'negative' : ''}>{sgn(c.mean, 3, true)}</td>
          <td>{c.lo == null ? '—' : `${fmt(c.lo, 3, true)}..${fmt(c.hi, 3, true)}`}</td>
          <td>{c.decisions == null ? 'unknown' : `${format(c.decisions)}${c.decisions > 0 ? ` · ${percent(c.n / c.decisions)}` : ''}`}</td>
          <td>{CLASS_STATE[c.state] ?? c.state}{filed?.ticket ? ` · ${filed.ticket}` : ''}</td>
        </tr>;
      })}</tbody>
    </table>
  </div>;
}

/** What the fleet found out about its own play (0273): the finding loop's own instruments, with
 * the evidence behind each and the ticket it filed. A finding that is a measurement says so — the
 * rule from 0269 is that a residual is not a loss, so only a decision cost is ever P0.
 *
 * #321: the panel states each family's explanation once instead of inside every row, and the
 * decision-cost instrument is a table rather than a paragraph per class. The coverage line — how much
 * of the window carries no usable evidence — leads, because it is the reason the rest says so little. */
function FleetFindings({training}: {training?:Training}) {
  const scan = training?.findings;
  const findings = scan?.findings || [];
  const classes = scan?.classes || [];
  const legends = scan?.legends || {};
  const coverage = scan?.coverage || [];
  const unanswered = scan?.unanswered || [];
  // The table is that instrument's own rows: a measurement row per class said in prose what the table
  // says in columns. A scan whose read failed serves no table, and those rows carry over from the last
  // pass — so they are listed as before rather than dropped.
  const rows = classes.length ? findings.filter(f => !f.id.startsWith('decision-')) : findings;
  // Grouped by the explanation they share, so a family states it once above its rows.
  const groups: {legend:string; items:Finding[]}[] = [];
  for (const f of rows) {
    const legend = legendFor(legends, f.id);
    const group = groups.find(g => g.legend === legend);
    if (group) group.items.push(f);
    else groups.push({legend, items: [f]});
  }
  if (!findings.length && !classes.length && !coverage.length && !unanswered.length) return null;
  return <section className="pc-panel fleet-findings" aria-label="What the fleet found about its own play">
    <h3>WHAT THE FLEET FOUND</h3>
    {coverage.length > 0 && <ul className="finding-coverage" role="status">{coverage.map((line, i) => <li key={i}>{line}</li>)}</ul>}
    {!rows.length && !classes.length && <p className="pc-note">Nothing above the floor: every decision class is under 0.02 bb per decision against the deep re-solve.</p>}
    {classes.length > 0 && <ClassTable classes={classes} legend={legendFor(legends, classes[0].id)} findings={findings}/>}
    {groups.map(g => <React.Fragment key={g.legend || 'rest'}>
      {g.legend && <p className="pc-note family-legend">{g.legend}</p>}
      <ul>{g.items.map(f => <li key={f.id} className={`finding-${f.severity.toLowerCase()}`}>
        <b>{f.severity}</b><span>{f.title}</span><small>{f.evidence}{f.ticket ? ` · filed as ${f.ticket}` : ''}</small>
      </li>)}</ul>
    </React.Fragment>)}
    {unanswered.length > 0 && <p className="pc-note">Could not measure: {unanswered.join('; ')}</p>}
    <p className="pc-legend">The loop runs every 30 minutes, files a ticket for anything new and serious, and closes that ticket when the class stops reproducing.</p>
  </section>;
}

export function Autonomy({training, onCommand, busy}: {training?:Training;onCommand:(command:string, extra?:object)=>void;busy:boolean}) {
  const progress = training?.progress;
  const nextJob = training?.next_job;
  const idleProgress = nextJob || progress;
  const idle = training?.status === 'idle';
  // Evidence refreshes run on the hands as they arrive (#314), so during play the learner is
  // nearly always refreshing: naming that beats reporting it as challenger validation, which is
  // what the status used to mean while a run was in flight.
  const refreshing = !idle && training?.job === 'refit';
  // A cooldown-paced search has no hand target to count towards: it waits on the clock, so the
  // panel says how long instead of showing an empty fraction.
  const pacedByClock = idle && !(idleProgress?.target || 0);
  const ratio = Math.min(100, (idleProgress?.hands || 0) / Math.max(1, idleProgress?.target || 20000) * 100);
  return <><div className="autonomy-orbit"><div className="orbit-ring"/><div className="orbit-core"><Cpu size={27}/></div><div><span className="eyebrow">STRATEGY ENGINE</span><strong>{training?.status === 'idle' ? (training?.automatic ? 'Autopilot scheduled' : 'Training paused') : training?.status === 'training' ? 'Training challenger' : 'Evaluating challenger'}</strong><span className="subtle">{training?.automatic ? 'Automatic improvement enabled' : 'Manual training mode'}</span></div></div>
    <p className="footnote" role="status">{training?.last_error ? `Retry scheduled: ${training.last_error}` : training?.status === 'idle' && training?.automatic ? `Next automatic ${nextJob?.kind === 'refit' ? 'refresh' : 'search'}: ${training.next_run && training.next_run > Date.now()/1000 ? time(training.next_run, { seconds: true }) : 'starting shortly'}` : 'Training runs automatically with no browser or button press required.'}</p>{training?.status === 'evaluating' && <p className="footnote amber">Clone pool of the live opponents{training.resumed ? ' · resumed' : ''}</p>}<div className="engine-checks"><span><ShieldCheck size={14}/>Legal action guard<span className="positive">ACTIVE</span></span><span><History size={14}/>Persistent learning<span className="positive">ACTIVE</span></span><span><FlaskConical size={14}/>Promotion gate<span className="amber">FRESH-DEAL 95%</span></span><span><Cpu size={14}/>Neural EV<span className={training?.neural_ev_status === 'available' ? 'positive' : 'amber'}>{training?.neural_ev_status === 'available' ? 'ACTIVE' : (training?.neural_ev_status || 'inactive').toUpperCase()}</span></span></div>
    <div className="training-progress"><div><span>{idle ? `NEXT JOB · ${(nextJob?.label || 'Champion search').toUpperCase()}` : refreshing ? 'EVIDENCE REFRESH' : 'CHALLENGER VALIDATION'}</span>{!pacedByClock && <b>{format(idle ? idleProgress?.hands || 0 : progress?.hands || 0)} <small>/ {format(idle ? idleProgress?.target || 0 : progress?.target || 20000)}</small></b>}</div>{!pacedByClock && <div className="progress-track"><div style={{width:`${ratio}%`}}/></div>}<p>{idle ? `${nextJob?.target ? `${format(nextJob.remaining || 0)} hands left. ` : ''}${nextJob?.reason || 'Waiting for the next scheduled learner job.'}${nextJob?.season_day ? ` · Season day ${nextJob.season_day}` : ''}` : refreshing ? 'Refitted on the hands as they arrive; live play reads these fits.' : 'Matched deals. Held-out seeds. Positive 95% lower bound required.'}</p>{!!progress?.challenger_decisions && <p>Learned policy used in {format(progress.challenger_policy_hits || 0)} / {format(progress.challenger_decisions)} challenger decisions · <span>{percent((progress.challenger_policy_hits || 0) / progress.challenger_decisions)} policy coverage</span>.</p>}{training?.next_candidate && !idle && <p>Next: {training.next_candidate.family} · {training.next_candidate.reason}</p>}</div>
    {!!training?.stale_loops?.length && <p className="footnote amber" role="alert">Stale loops — learning is degraded until these report: {training.stale_loops.map(l => `${l.name} (${l.message})`).join(' · ')}</p>}
    <FleetFindings training={training}/>
    {!!training?.learning?.length && <div className="learned-recently" aria-label="Learned recently"><span className="eyebrow">LEARNED RECENTLY</span><ul>{training.learning.map(item => <li key={item.what}><div><b>{item.what}</b><span>{item.updated ? ago(item.updated) : 'not yet'}</span></div><p>{item.detail}</p></li>)}</ul></div>}
    <CooldownSetting training={training}/>
    <button className="button full" disabled={busy} onClick={() => onCommand(training?.status === 'idle' ? 'start' : 'stop')}>{training?.status === 'idle' ? <Play size={13}/> : <Square size={13}/>} {training?.status === 'idle' ? (training?.automatic ? 'Start next search early' : 'Run one search') : 'Cancel this job'}</button>
  </>;
}

/** The key the learner counts a death under, as a phrase: `search/no-effect` -> `search · no effect`. */
const funnelLabel = (key:string) => key.replace('/', ' · ').replaceAll('-', ' ');

/** Candidate proposals and their outcomes are separate counts, not additive candidates. */
export function SearchFunnel({funnel}: {funnel:SearchFunnel}) {
  const proposed = funnel.outcomes.find(o => o.key === 'search/proposed')?.count ?? 0;
  return <div className="funnel">
    <div className="funnel-head"><span>Strategy search · last {funnel.hours}h</span><b>{format(proposed)} proposed</b></div>
    <div className="funnel-chips">{funnel.outcomes.map(o=><span className={`funnel-chip ${o.key.split('/')[0]}`} key={o.key} title={o.key}>{funnelLabel(o.key)}<b>{format(o.count)}</b></span>)}</div>
    {funnel.knobs.length > 0 && <div className="funnel-knobs">Knobs: {funnel.knobs.slice(0,6).map(k=>`${k.key.replaceAll('_',' ')} ${format(k.count)}`).join(' · ')}</div>}
    <p className="footnote">Proposals and outcomes are separate counts. A rejected challenger keeps the current champion; opponent models and calibration keep learning.</p>
  </div>;
}

/** Keep the latest successful change visible after it leaves the recent candidate list. */
export function RecentExperiments({training}: {training:Training}) {
  const latest = training.last_promotion;
  const recent = training.experiments.slice(0,6);
  const pinned = latest?.id && latest.status === 'promoted' && typeof latest.ts === 'number' && !recent.some(e => e.id === latest.id);
  return <>
    {pinned && <><span className="eyebrow">LATEST SUCCESSFUL CHANGE</span><ExperimentCard experiment={latest as Experiment}/></>}
    <p className="footnote">Latest {recent.length} candidate decisions · {training.experiments.length} retained. Model learning runs independently of champion search.</p>
    <div className="experiments">{recent.map(experiment => <ExperimentCard experiment={experiment} key={experiment.id}/>)}</div>
  </>;
}

export function ExperimentCard({experiment}: {experiment:Experiment}) {
  const synthetic = experiment.strata?.synthetic;
  const observed = experiment.strata?.observed;
  const interval = (lower?:number|null, upper?:number|null) => lower == null || upper == null ? 'awaiting evidence' : `${signed(lower*100,1)} to ${signed(upper*100,1)} bb/100`;
  return <div className="experiment"><span className={`experiment-dot ${experiment.status === 'promoted' ? 'promoted' : ''}`}/><div><div><b>{experiment.knob?.replaceAll('_',' ') || 'Strategy evaluation'}</b><span className="tag">{experiment.status}</span></div><p>{experiment.old} <span>→</span> {experiment.new}</p>{experiment.rationale && <p className="footnote">{experiment.rationale}</p>}{experiment.population && <small>{format(experiment.population.opponent_count)} opponents · {format(experiment.population.evidence)} observations · model {experiment.population.id}</small>}{experiment.strata ? <div className="experiment-strata">{synthetic && <small>Synthetic {format(synthetic.hands)} · {interval(synthetic.lower_95,synthetic.upper_95)}</small>}<small>Clone pool {format(observed?.hands)} hands · {interval(observed?.lower_95,observed?.upper_95)}</small></div> : <small>{format(experiment.hands)} hands · {interval(experiment.lower_95,experiment.upper_95)}</small>}{experiment.terminal_reason && <p className="footnote">{experiment.terminal_reason}</p>}</div></div>;
}

export function Season({bot}: {bot?:Bot}) {
  const season = bot?.season || {};
  const numeric = (key:string) => typeof season[key] === 'number' ? season[key] as number : null;
  return <Panel title="Season ledger" icon={<Layers size={15}/>} aside={<span className="tag">{numeric('rank') ? `RANK #${numeric('rank')}` : 'UNRANKED'}</span>}><div className="health-list"><div><span>Season score</span><b className="amber">{format(numeric('score'))}</b></div><div><span>Off-table chips</span><b>{format(numeric('chip_balance'))}</b></div><div><span>Chips at table</span><b>{format(numeric('chips_at_table'))}</b></div><div><span>Rebuys · penalty</span><b className={numeric('rebuy_penalty') === 0 && (numeric('chip_balance') || 0) + (numeric('chips_at_table') || 0) < (numeric('starting_chips') || 5000) ? 'amber' : ''} title={numeric('rebuy_penalty') === 0 ? 'Rebuy penalty 0: below the starting stack, busting costs nothing extra, so variance is cheap' : undefined}>{format(numeric('rebuys'))} · {format(numeric('rebuy_penalty'))}</b></div><div><span>Season hands</span><b>{format(numeric('hands_played'))}</b></div></div><p className="footnote">Server-confirmed season totals. Chip balances and score are separate from hand winnings.</p></Panel>;
}

/** One strategy parameter, as the server defines it: the search's own bounds are the track, the
 *  shipped default is the mark to read the value against, and the value is the champion's.
 *
 *  #322: a value the payload did not carry used to fall back to the knob's minimum, so the bar sat
 *  empty at the left — "pinned to its floor" — beside a printed `—` saying it was unknown. The two
 *  halves of the cell disagreed and the bar was the half that lied. Unknown now draws no bar at all. */
function KnobRow({knob, value}: {knob:ChampionKnob; value:number|null}) {
  const span = knob.max - knob.min;
  const at = (v:number) => `${span > 0 ? Math.max(0, Math.min(100, (v - knob.min) / span * 100)) : 0}%`;
  const shown = value == null ? '—' : value.toFixed(knob.decimals);
  const range = `${knob.min.toFixed(knob.decimals)} to ${knob.max.toFixed(knob.decimals)}`;
  return <div className={`knob${value == null ? ' knob-unknown' : ''}`}>
    <div><label>{knob.label}</label><b>{shown}</b></div>
    <div className="knob-track" role="img" aria-label={value == null ? `${knob.label}: not reported` : `${knob.label}: ${shown} of ${range}`}>
      {value != null && <i style={{width:at(value)}}/>}
      <span className="knob-default" style={{left:at(knob.default)}} title={`Default ${knob.default.toFixed(knob.decimals)} · searched over ${range}`}/>
    </div>
    <p className="knob-description">{knob.description}</p>
  </div>;
}

export function Profile({training, bot}: {training?:Training;bot?:Bot}) {
  // SvanBot's live strategy parameters (the learner promotes changes to these after paired evaluation).
  // The rows are the server's own knob catalogue (#322): the panel used to hold a hand-typed copy of
  // the list, its bounds and its explanations, so a knob the search gained never appeared here and a
  // bound it widened left this bar measuring against the old one.
  const knobs = training?.knobs || [];
  const value = (key:string) => { const raw = training?.champion[key]; return typeof raw === 'number' ? raw : null; };
  return <div className="profile"><div className="profile-version"><span className="status-dot"/> {training?.champion.name || training?.champion.version || 'sv10-ev-1'}<span className="tag">CHAMPION</span></div>{knobs.map(knob => <KnobRow key={knob.key} knob={knob} value={value(knob.key)}/>)}{!knobs.length && <p className="footnote">No parameter list from the server, so there is nothing to draw: the running server is what defines the knobs and the ranges they are searched over.</p>}<dl className="champion-details"><dt>Opponent profiles tracked</dt><dd>{format(training?.opponent_profiles_tracked)}</dd><dt>Past-season hands</dt><dd>{training?.past_hands ? `${format(training.past_hands.downloaded)} downloaded of ${format(Object.values(training.past_hands.bots || {}).reduce((sum,b)=>sum+(b.server_total||0),0))}` : 'Download pending'}</dd><dt>Range model</dt><dd>{training?.range_model ? `${training.range_model.active ? 'Showdown-fitted' : 'Defaults (fit not better)'} · ${format(training.range_model.showdowns)} showdowns · ${format(training.range_model.gain_fitted,2)} vs ${format(training.range_model.gain_default,2)} nats` : 'Hand-set defaults'}</dd><dt>Strength tables</dt><dd>{training?.storage?.tables ? `${training.storage.tables.flop ? 'flop ✓' : 'flop missing'} · ${training.storage.tables.turn ? 'turn ✓' : 'turn missing'}` : '—'}</dd><dt>Data integrity</dt><dd>{training?.storage?.checked_at ? `${training.storage.database_check} · ${format(training.storage.digests_checked)} hand digests, ${format(training.storage.digest_mismatches)} mismatched · backup ${training.storage.last_backup ?? '—'}${training.storage.archive_latest ? ` · archive ${training.storage.archive_latest}` : ''}` : 'Checked hourly'}</dd><dt>Extra hand data</dt><dd>{training?.storage?.corpus && Object.keys(training.storage.corpus).length ? Object.entries(training.storage.corpus).map(([k,v])=>`${k}: ${format(v)}`).join(' · ') : 'None imported'}</dd><dt>Promotion lineage</dt><dd>{training?.lineage?.join(' → ') || training?.champion.version || 'sv10-ev-1'}</dd><dt>Version for this hand</dt><dd>{bot?.version || "No active hand"}</dd><dt>Latest decision source</dt><dd>{bot?.decision?.source || "Awaiting a decision"}</dd></dl><p className="footnote">Live parameters of the exploitative EV policy. The learner changes them only after a challenger wins a paired simulation against clones of the live opponent pool with a positive 95% lower bound.</p><p className="footnote">Promoted parameters reach every connected seat within 30 seconds.</p></div>;
}

/** Experiment mode (0291): when the fleet holds #1–#4, the bot at #4 and the fifth bot test one
 * learner challenger live, alternating treatment and champion control. Nothing here claims an
 * experiment until a real target is installed; live evidence never promotes. */
export function ExperimentModePanel() {
  const poll = usePoll<ExperimentMode>('/experiment', 30000);
  const x = poll.data;
  if (!x) return <><p className="footnote">{poll.error ? `Unavailable: ${poll.error}` : 'Loading experiment mode…'}</p></>;
  const age = x.reading_age_secs;
  const fresh = age != null && age <= x.stale_after_secs;
  const est = x.target?.estimate;
  const headline = x.status === 'running' ? 'Testing a challenger live' : x.status === 'active_no_target' ? 'Top four held · no safe target' : 'Champion everywhere';
  return <div className="experiment-mode">
    <div className="two-stats"><div><label>MODE</label><b className={x.status === 'running' ? 'positive' : x.status === 'active_no_target' ? 'amber' : ''}>{headline}</b></div><div><label>QUALIFYING READINGS</label><b>{x.status === 'champion' ? `${x.qualifying.readings} / ${x.qualifying.needed}` : 'qualified'}</b></div></div>
    <p className="footnote">Season {x.season ?? 'unknown'} · official score board every 5 min · {age == null ? 'no reading yet' : `last reading ${ago(Date.now()/1000 - age)}`}{age != null && !fresh ? ' · stale: champion' : ''}{x.last_error ? ` · last poll failed: ${x.last_error}` : ''}</p>
    {x.reason && <p className="footnote amber" role="status">{x.reason}</p>}
    {x.protected.length > 0 && <p className="footnote">Protected (champion only): {x.protected.map(n => `${n} #${x.ranks[n] ?? '?'}`).join(' · ')}</p>}
    {x.bots.length > 0 && <div className="health-list">{x.bots.map(b => <div key={b.name}><span><PlayerName name={b.name}/> #{x.ranks[b.name] ?? '?'}</span><b className={b.role === 'treatment' ? 'amber' : ''}>{b.role ?? 'champion'}{x.target ? ` · ${format(b.hands_on_target)} hands` : ''}</b></div>)}</div>}
    {x.target && <div className="training-progress"><div><span>TARGET · {x.target.label.toUpperCase()}</span><b>{format(est?.effective_hands || 0)} <small>/ {format(x.target.required_hands)} per arm</small></b></div><div className="progress-track"><div style={{width:`${Math.min(100, (est?.effective_hands || 0) / x.target.required_hands * 100)}%`}}/></div>
      <p>{x.target.hypothesis} · {x.target.source === 'confirmation' ? 'in fresh-deal confirmation' : 'undecided in simulation'} (sim {signed(x.target.sim.mean_bb100, 1)} bb/100, upper {signed(x.target.sim.upper_bb100, 1)}) · vs {x.target.champion}</p>
      {est && <p>Live: treatment − control {signed(est.diff_bb100, 1)} bb/100 (95% {signed(est.lower_bb100, 1)}..{signed(est.upper_bb100, 1)}) · {format(est.treatment.hands)} treatment / {format(est.control.hands)} control hands</p>}
      <p>Next gate: {x.target.next_gate ?? 'waiting for hands'} · {x.target.safety_bound}. Only the fresh-deal gate promotes. Audit: <code>{x.target.review}</code></p></div>}
    {x.verdicts.length > 0 && <ul className="footnote">{x.verdicts.map(v => <li key={v.id}><b>{v.verdict}</b> · {v.label} · {signed(v.estimate.diff_bb100, 1)} bb/100 over {format(v.estimate.effective_hands)} hands per arm · {ago(v.at)}</li>)}</ul>}
    {x.last_transition && <p className="footnote">Last switch {time(x.last_transition.at, { seconds: true })}: {x.last_transition.reason}</p>}
    <StaleNote poll={poll}/>
  </div>;
}
