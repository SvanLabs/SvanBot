import { BookOpen, CircleHelp, Gauge, Pause, Play, Settings2, Spade, Square, Target, Tv } from 'lucide-react';
import type { AccuracyState, Bot, ExperimentMode, Training } from './types';
import { SoundToggle } from './sound';
import { usePoll } from './api';
import { fmt, share, time, TURN_DEADLINE_S } from './format';
import { EvBars, chosenOption, moveLabel, planSentence } from './fun';

const unavailable = 'Unavailable';
const sentence = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);
const sourceLabel = (source: string) => sentence(source.replaceAll('-', ' ').replace(/\bev\b/gi, 'EV').replace(/\bnn\b/gi, 'NN'));

export function DashboardHeader({bots, selectedSlot, connected, onSelect, onSettings}: {
  bots: Bot[];
  selectedSlot?: number;
  connected: boolean;
  onSelect: (slot: number) => void;
  onSettings: () => void;
}) {
  return <header className="topbar">
    <a className="brand" href="/" aria-label="SvanBot home">
      <span className="brand-mark"><Spade size={20} fill="currentColor"/><i>10</i></span>
      <span>SVAN<span className="brand-light">BOT</span><small>CONTROL ROOM / V10</small></span>
    </a>
    <div className="header-divider"/>
    <nav className="bot-tabs" aria-label="Bot selection">{bots.map(bot =>
      <button aria-label={`${bot.name}, ${bot.connected ? 'online' : 'offline'}`} className={bot.slot === selectedSlot ? 'selected' : ''} key={bot.slot} onClick={() => onSelect(bot.slot)}>
        <span className={`status-dot ${bot.connected ? '' : 'offline'}`}/>{bot.name}<span className="tab-number">0{bot.slot}</span>
      </button>
    )}</nav>
    <div className="header-right">
      <a className="text-button" href="#docs" aria-label="Documentation"><BookOpen size={17}/>Docs</a>
      <a className="text-button" href="#help" aria-label="Bot help"><CircleHelp size={17}/>Help</a>
      <a className="text-button" href="#tv" aria-label="TV mode: auto-directed live table with commentary"><Tv size={17}/>TV</a>
      <a className="text-button" href="#quiz" aria-label="Quiz: what would Svanbot do?"><Target size={17}/>Quiz</a>
      <span className="system-status"><span className={`status-dot ${connected ? '' : 'offline'}`}/>{connected ? 'SYSTEM ONLINE' : 'CONNECTING'}</span>
      <SoundToggle selectedSlot={selectedSlot} bigBlind={bots.find(b => b.slot === selectedSlot)?.big_blind || 20}/>
      <button className="icon-button" title="Settings" aria-label="Open settings" onClick={onSettings}><Settings2 size={17}/></button>
    </div>
  </header>;
}

/** `raise:1375` / `all_in` as the analyst's action strings read: `raise 1,375`, `all in`. */
const moveText = (action: string) => action.replace(':', ' ').replaceAll('_', ' ');

export function DecisionTelemetry({bot, training}: {bot?: Bot; training?: Training}) {
  const decision = bot?.decision;
  const equity = decision?.equity;
  // The arm and the grading are not part of the state payload: the experiment view says which arm a
  // bot is in, and the accuracy report carries the analyst's grade for a decision once it is graded.
  const experiment = usePoll<ExperimentMode>('/experiment', 60_000);
  const grading = usePoll<AccuracyState>('/accuracy', 120_000);
  const latency = decision?.latency_ms;
  const timing = typeof latency === 'number' ? `${fmt(latency)} ms` : unavailable;
  // The server auto-acts 45 s after `your_turn`; a decision is a share of that clock.
  const deadlineTitle = typeof latency === 'number' ? `${fmt(latency / 1000, 2)} s of the ${TURN_DEADLINE_S} s turn deadline` : undefined;
  const deadlineNote = typeof latency === 'number'
    ? `${fmt(latency / 1000 / TURN_DEADLINE_S * 100, 1)}% of the ${TURN_DEADLINE_S} s turn deadline · ${fmt(Math.max(0, TURN_DEADLINE_S - latency / 1000), 1)} s to spare`
    : null;
  const equityMetricLabel = equity?.exact ? 'EQUITY COMPUTATION' : 'SAMPLING ERROR';
  const equityMetric = equity?.exact
    ? 'Exact for modeled range'
    : typeof equity?.standard_error === 'number'
      ? `${fmt(equity.standard_error * 100, 2)} pp SE`
      : unavailable;
  const equityEvidence = equity?.exact
    ? equity.samples != null ? `${fmt(equity.samples)} modeled outcomes` : 'Modeled range'
    : equity?.samples != null ? `Monte Carlo · ${fmt(equity.samples)} samples` : 'No sampling error reported';
  const source = decision?.source ? sourceLabel(decision.source) : unavailable;
  const trainingState = training ? `${sentence(training.status.replaceAll('_', ' '))} · ${training.champion.version}` : unavailable;
  // The plan, the option table and the price are all in the decision record (0299).
  const plan = planSentence(decision);
  const { cands, chosen } = chosenOption(decision);
  const bets = cands.filter(c => (c.fold_prob ?? 0) > 0).sort((a, b) => (b.fold_prob ?? 0) - (a.fold_prob ?? 0));
  const bestBet = chosen && (chosen.action === 'raise' || chosen.action === 'all_in') && (chosen.fold_prob ?? 0) > 0 ? chosen : bets[0];
  const eq = equity?.value ?? null;
  const price = decision?.pot_odds ?? null;
  const toCall = decision?.to_call ?? 0;
  // A recorded arm (when the server puts one on the decision) beats the live experiment view, which
  // names the arm this bot is playing right now.
  const recordedArm = ['arm', 'role', 'label', 'name', 'id']
    .map(key => decision?.experiment?.[key])
    .filter((value): value is string => typeof value === 'string' && value.length > 0)
    .join(' · ');
  const liveArm = experiment.data?.status === 'running' ? (experiment.data.bots ?? []).find(b => b.name === bot?.name) : undefined;
  const arm = recordedArm || liveArm?.role || null;
  const graded = decision && bot?.hand_id
    ? (grading.data?.worst ?? []).find(w => w.hand_id === bot.hand_id && w.bot === bot.name && (!decision.street || w.street === decision.street))
    : undefined;
  const priceCaption = toCall > 0 && eq != null && price != null
    ? `A ${fmt(toCall)}-chip call needs ${share(price)} of the pot and this hand has ${share(eq)} — ${eq >= price ? 'above' : 'below'} the price.`
    : eq != null && price != null
      ? 'Nothing to call: the policy acts for free, so the price is 0%.'
      : 'This record carries no equity or price.';

  return <section className="telemetry-rail" aria-label="Decision telemetry">
    <div className="telemetry-head">
      <div className="telemetry-intro"><Gauge size={15}/><span><b>SV10 SIGNAL</b><small>Evidence carried with every decision</small></span></div>
      <dl>
        <div title={deadlineTitle}><dt>DECISION TIME</dt><dd>{timing}</dd><small>{deadlineNote ?? 'Latest completed action'}</small></div>
        <div><dt>{equityMetricLabel}</dt><dd>{equityMetric}</dd><small>{equityEvidence}</small></div>
        <div><dt>STRATEGY SOURCE</dt><dd>{source}</dd><small>{decision?.version || bot?.version || 'No active hand'}</small></div>
        <div><dt>TRAINING</dt><dd>{trainingState}</dd><small>{training?.automatic ? 'Automatic schedule' : training ? 'Manual schedule' : 'No status reported'}</small></div>
      </dl>
    </div>
    {decision && <div className="signal-detail">
      <div className="signal-plan">
        {plan && <p className="signal-sentence" title={plan.raw && plan.raw !== plan.text ? `Server shorthand: ${plan.raw}` : undefined}>{plan.text}</p>}
        <EvBars decision={decision}/>
      </div>
      <div className="signal-facts">
        <div className="signal-fact">
          <span className="fact-label">EQUITY vs POT ODDS</span>
          {eq != null && price != null && <span className="price-gauge" role="img" aria-label={`Equity ${share(eq)} against a call price of ${share(price)}`}>
            <i className="price-fill" style={{width: `${Math.min(100, Math.max(0, eq * 100))}%`}}/>
            <i className="price-mark" style={{left: `${Math.min(100, Math.max(0, price * 100))}%`}}/>
          </span>}
          <b>{eq == null ? unavailable : share(eq)} <span className="fact-unit">equity</span> · {price == null ? unavailable : share(price)} <span className="fact-unit">call price</span></b>
          <small>{priceCaption}</small>
        </div>
        <div className="signal-fact">
          <span className="fact-label">THEIR FOLD CHANCE</span>
          <b>{bestBet?.fold_prob ? share(bestBet.fold_prob) : 'None priced'}</b>
          <small>{bestBet?.fold_prob
            ? `${moveLabel(bestBet)}${bestBet === chosen ? ' — the move it took' : ''} · the chance every opponent still in the hand folds, from the same response model the EV uses`
            : 'No bet was among the options this decision priced.'}</small>
        </div>
        <div className="signal-fact">
          <span className="fact-label">EXPERIMENT ARM</span>
          <b className={arm ? 'amber' : ''}>{arm ?? (experiment.data ? 'Champion everywhere' : unavailable)}</b>
          <small>{recordedArm ? 'Recorded on this decision'
            : liveArm ? `Live now · ${fmt(liveArm.hands_on_target)} hands on target · pair ${(experiment.data?.pair ?? []).join(' vs ')}`
              : experiment.data ? ((experiment.data.protected ?? []).includes(bot?.name || '') ? 'Protected: the fleet holds the top four, so this bot plays the champion' : 'No experiment arm is live for this bot') : (experiment.error ?? 'Experiment mode not reported')}</small>
        </div>
        {graded && <div className="signal-fact signal-graded">
          <span className="fact-label">DEEP RE-SOLVE</span>
          <b><i className={`gr-chip gr-${graded.grade}`}>{sentence(graded.grade)}</i>{moveText(graded.deep_action)} was best</b>
          <small>live {moveText(graded.live_action)} gave up {fmt(graded.loss_bb, 1)} bb of a {fmt(graded.pot_bb, 0)} bb pot · {graded.street} · graded {time(graded.ts, { seconds: true })}</small>
        </div>}
      </div>
    </div>}
  </section>;
}

export function BotControls({bot, busy, connected, onCommand}: {
  bot?: Bot;
  busy: boolean;
  connected: boolean;
  onCommand: (command: 'start' | 'pause' | 'stop') => void;
}) {
  const playing = bot?.mode === 'playing' || bot?.mode === 'pausing';
  return <div className="bot-controls" aria-label="Bot controls">
    <button className="button primary" disabled={busy || !bot || playing || !connected} onClick={() => onCommand('start')}><Play size={12} fill="currentColor"/>Start</button>
    <button className="button" disabled={busy || !playing || !connected} onClick={() => onCommand('pause')}><Pause size={12}/>{bot?.mode === 'pausing' ? 'Pausing…' : 'Pause'}</button>
    <button className="button" disabled={busy || !bot || bot.mode === 'stopped' || !connected} onClick={() => onCommand('stop')}><Square size={11}/>Stop</button>
  </div>;
}
