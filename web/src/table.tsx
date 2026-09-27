/** The live poker table, TV mode and the decision strip. */
import React, { useEffect, useRef, useState } from 'react';
import { Spade, Users, Zap } from 'lucide-react';
import { commentary, directorScore, nextShot } from './director';
import { openPlayerCard } from './playercard';
import { planSentence } from './fun';
import type { Bot, Decision, TableBot } from './types';
import { format, percent, readLine, readTitle, Card } from './ui';
import type { TableTheme } from './ui';
import { PlayerName } from './playername';

/** Chip denominations in chips, largest first; the colour is the value, as on a real table. */
const CHIP_VALUES = [1000, 500, 100, 25, 5, 1];
const MAX_CHIPS = 6;

/** How an amount stacks: at most six chips, largest first, so a big bet reads as a tall stack and
 *  the exact figure stays in the number printed beside it. */
function chipBreakdown(amount: number): number[] {
  const chips: number[] = [];
  let left = Math.max(0, Math.round(amount));
  for (const value of CHIP_VALUES) {
    while (left >= value && chips.length < MAX_CHIPS) { chips.push(value); left -= value; }
  }
  return chips;
}

/** The chips an amount is made of, bottom-up, in the stack the CSS expects (`--stack-i`).
 *  Decorative: the amount is always printed beside it and it is hidden from assistive tech. */
function Chips({amount, className}: {amount: number; className?: string}) {
  const chips = chipBreakdown(amount);
  if (!chips.length) return null;
  return <span className={`chip-stack ${className || ''}`} aria-hidden="true">
    {chips.map((value, index) => <i key={index} className={`chip chip-${value}`} style={{'--stack-i': index} as React.CSSProperties}/>)}
  </span>;
}

/** `interactive: false` is the public TV: the seat plates stop being scouting-report buttons. The
 *  scout view reads the operator's opponent profiles, and a spectator on the public listener has no
 *  route to them — so the affordance is not offered rather than offered and refused. */
export function PokerTable({bot, theme, interactive = true}: {bot?: TableBot; theme: TableTheme; interactive?: boolean}) {
  const seats = Array.from({length:6}, (_, index) => bot?.seats.find(seat => seat.seat === index));
  const occupied = seats.filter(seat => seat?.name).length;
  const bigBlind = bot?.big_blind || 20;
  const blinds = (chips: number) => format(chips / bigBlind, chips / bigBlind >= 1000 ? 0 : 1);
  const [winners, setWinners] = useState<string[]>([]);
  useEffect(() => {
    setWinners([]);
    let timer: number | undefined;
    const onResult = (event: Event) => {
      const result = (event as CustomEvent<{type: string; slot: number; hand_id?: string; winners?: string[]}>).detail;
      if (result.type !== 'result' || result.slot !== bot?.slot || result.hand_id !== bot?.hand_id) return;
      clearTimeout(timer);
      setWinners(result.winners || []);
      timer = window.setTimeout(() => setWinners([]), 3500);
    };
    window.addEventListener('sv-live', onResult);
    return () => { clearTimeout(timer); window.removeEventListener('sv-live', onResult); };
  }, [bot?.slot, bot?.hand_id]);
  return <div className={`table-stage table-theme-${theme}`}>
    <div className="table-ambient"/>
    <div className="poker-rail"><div className="poker-felt"><div className="felt-line"/>
      <div className="board-area">
        <div className="pot"><span>POT</span><b key={bot?.pot || 0} className={bot?.pot ? 'pot-bump' : ''}>{format(bot?.pot || 0)}</b><small>{blinds(bot?.pot || 0)} BB</small><Chips key={`pot-${bot?.pot || 0}`} className="pot-chips" amount={bot?.pot || 0}/></div>
        <div className="board-cards">{Array.from({length:5}, (_, index) => <span className="card-slot" key={`${bot?.hand_id}-${index}`}><span className={bot?.board[index] ? 'board-deal' : ''} key={bot?.board[index] || 'empty'} style={{animationDelay: `${index < 3 ? index * 90 : 0}ms`}}>{bot?.board[index] && <Card card={bot.board[index]}/>}</span></span>)}</div>
        <div className="felt-brand"><Spade/> SVANBOT <span>NO LIMIT HOLD’EM · {format(bigBlind / 2)}/{format(bigBlind)} · {occupied} SEATED</span></div>
      </div>
      {seats.map((seat, index) => seat?.name ? <div key={index} className={`bet-spot spot-${index} ${seat.bet ? 'filled' : ''}`} aria-label={`${seat.name} bet ${format(seat.bet || 0)}`}>
        {!!seat.bet && <><Chips key={`${bot?.hand_id}-${bot?.street}-${seat.bet}`} amount={seat.bet}/><b>{format(seat.bet)}</b></>}
        {seat.seat === bot?.dealer_seat && <span className="dealer" title="Dealer button">D</span>}
      </div> : null)}
    </div></div>
    {seats.map((seat, index) => {
      const hero = !!seat && seat.seat === bot?.hero_seat;
      const acting = !!seat && seat.seat === bot?.actor_seat && winners.length === 0;
      if (!seat?.name) return <div key={index} className={`seat seat-${index} unoccupied`}><div className="seat-plate"><span className="avatar"><Users size={14}/></span><div className="seat-info"><span className="seat-name">Seat {index + 1}</span><small>Open</small></div></div></div>;
      const winner = !!seat.name && winners.includes(seat.name);
      const allIn = !seat.folded && seat.last_action === 'all_in';
      // The scout view is the operator's own read on a player. On the public TV the plate is a
      // label: the affordance is not offered, rather than offered and then refused by a 404.
      const scout = interactive ? {
        role: 'button' as const,
        tabIndex: 0,
        title: hero ? `Our seat: ${seat.name}` : `Scouting report: ${seat.name}`,
        'aria-label': `Open scout view: ${seat.name}`,
        onClick: () => openPlayerCard(seat.name!),
        onKeyDown: (e: React.KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openPlayerCard(seat.name!); } },
      } : {};
      return <div key={index} className={`seat seat-${index} ${hero ? 'hero' : ''} ${acting ? 'acting' : ''} ${seat.folded ? 'folded' : ''} ${winner ? 'winner' : ''} ${allIn ? 'all-in' : ''}`}>
        {/* Kept mounted while folded (and mucked away by CSS): the cards have to exist for the fold
            to animate. On the dashboard the hero's own cards stay readable either way; the public
            TV's payload never carries `hole`, so there they are face down — a spectator's view of a
            seat, which is what the projection is for. */}
        <div className="seat-cards" key={`${bot?.hand_id}-${seat.name}`}><Card small card={hero ? bot?.hole?.[0] : undefined}/><Card small card={hero ? bot?.hole?.[1] : undefined}/></div>
        {winner && <span className="payout-chips" aria-hidden="true"><i/><i/><i/></span>}
        <div className={`seat-plate ${interactive ? 'scoutable' : ''}`} {...scout}>{winner && <span className="winner-shine" aria-hidden="true"/>}<span className="avatar">{acting && <span className="turn-ring" key={`${bot?.hand_id}-${bot?.street}-${bot?.pot}`} aria-hidden="true"/>}{seat.avatar_url ? <img className="avatar-img" src={seat.avatar_url} alt="" referrerPolicy="no-referrer" loading="lazy" onError={e => { e.currentTarget.style.display = 'none'; }}/> : null}<span className="avatar-initials">{seat.name.slice(0,2).toUpperCase()}</span></span><div className="seat-info"><span className="seat-name">{seat.name}{hero && <em>YOU</em>}</span><b>{format(seat.stack)}</b><small>{blinds(seat.stack)} BB</small>{seat.read && <small className="seat-read" title={readTitle(seat.read)}>{readLine(seat.read)}</small>}</div></div>
        <span key={`${bot?.hand_id}-${seat.last_action}-${acting}-${winner}`} className="seat-action">{winner ? 'Winner' : seat.folded ? 'Folded' : acting ? 'To act' : seat.last_action?.replace('_',' ') || 'In hand'}</span>
      </div>;
    })}
  </div>;
}

/** TV mode (0177): one full-screen table chosen by the auto-director, with play-by-play commentary
 * built from each decision's reason and equity. Presentation only.
 *
 * `public` is the same view on the unauthenticated listener (`SVANBOT_TV_PORT`), where the audience
 * has no operator token. Three things change: the seat plates are not scouting-report buttons, there
 * is no dashboard to exit back to, and the commentary stays empty — it is built from decision events
 * carrying each decision's equity, and the public stream deliberately does not carry them. */
export function TvMode({bots, theme, public: onPublicTv = false}: {bots: TableBot[]; theme: TableTheme; public?: boolean}) {
  const [slot, setSlot] = useState<number>();
  const since = useRef(Date.now());
  const [lines, setLines] = useState<{id:number;text:string}[]>([]);
  const counter = useRef(0);
  const slotRef = useRef(slot);
  slotRef.current = slot;
  useEffect(() => {
    const next = nextShot(bots, slot, Date.now() - since.current);
    if (next !== slot) { setSlot(next); since.current = Date.now(); }
  }, [bots, slot]);
  useEffect(() => {
    const onEvent = (e: Event) => {
      const ev = (e as CustomEvent<{type:string;slot:number;bot?:string;action?:string;amount?:number|null;equity?:number;net?:number|null}>).detail;
      if (ev.slot !== slotRef.current) return;
      const text = commentary(ev);
      if (text) setLines(list => [{id: ++counter.current, text}, ...list].slice(0, 4));
    };
    window.addEventListener('sv-live', onEvent);
    return () => window.removeEventListener('sv-live', onEvent);
  }, []);
  const bot = bots.find(b => b.slot === slot);
  return <div className="tv-mode" role="region" aria-label="TV mode">
    <div className="tv-top"><span className="tv-live"><span className="status-dot"/>LIVE</span><b>{bot?.name ?? 'Waiting for a table'}</b>
      <span className="tv-meta">{bot ? `pot ${format(bot.pot)} · ${format((bot.pot || 0) / (bot.big_blind || 20), 0)} bb` : ''}</span>
      {!onPublicTv && <a className="text-button" href="#">Exit TV</a>}</div>
    <div className="tv-stage" key={slot}><PokerTable bot={bot} theme={theme} interactive={!onPublicTv}/></div>
    {!onPublicTv && <ol className="tv-commentary" aria-live="polite">{lines.map((l, i) => <li key={l.id} className={i === 0 ? 'fresh' : ''}>{l.text}</li>)}</ol>}
    <div className="tv-pip">{bots.filter(b => b.slot !== slot).map(b => <button key={b.slot} onClick={() => { setSlot(b.slot); since.current = Date.now(); }} className={directorScore(b) >= 1000 ? 'hot' : ''}>
      <span className={`status-dot ${b.connected ? '' : 'offline'}`}/>{b.name}<small>{b.hand_id ? `${format((b.pot || 0) / (b.big_blind || 20), 0)} bb` : 'idle'}</small></button>)}</div>
  </div>;
}

export function DecisionStrip({decision, bot}: {decision?:Decision | null;bot?:Bot}) {
  // The reason reads as a sentence here; the record's own shorthand (and the option dump the signal
  // strip now draws) stays in the tooltip (0299).
  const plan = planSentence(decision);
  return <div className="decision-strip"><div className="decision-primary"><div className="hero-cards"><Card small card={bot?.hole[0]}/><Card small card={bot?.hole[1]}/></div><div><div className="decision-title">{decision ? <b className={decision.action === 'fold' ? 'muted' : 'amber'}>{decision.action.replace('_',' ').toUpperCase()}</b> : <b>READY WHEN YOU ARE</b>}<span className="tag">{decision?.source || 'standby'}</span></div><p title={decision?.reason || undefined}>{plan?.text || 'Start a bot to connect. Every action and its reasoning will appear here.'}</p></div><span className="decision-latency"><Zap size={13}/>{decision?.latency_ms == null ? 'Unavailable' : `${format(decision.latency_ms)} ms`}</span></div>
    {decision?.hand_category && <div className="hand-analysis"><b>{decision.hand_category}</b><span>Best five</span><div className="inline-cards">{decision.best_five?.map(card=><Card key={card} small card={card}/>)}</div></div>}
    {decision && bot?.street === 'preflop' && decision.preflop_score != null && <p className="footnote">Opening guide{decision.opening_position ? ` · ${decision.opening_position}` : ''} · hand score {format(decision.preflop_score,2)}{decision.opening_threshold != null ? ` / weakest opened ${format(decision.opening_threshold,2)} · ${decision.preflop_score >= decision.opening_threshold ? 'inside' : 'outside'} the guide` : ' · guide warming up'} · live EV policy decides, first-in guide</p>}
    {decision?.effective_stack != null && <p className="footnote">Effective remaining stack: {format(decision.effective_stack,1)} BB · stack / pot: {format(decision.spr,2)}</p>}
    {!!decision?.opponent_models?.length && <details className="opponent-model-inputs"><summary>Opponent model inputs · {decision.opponent_models.length}</summary>{decision.opponent_models.map(model=><p key={model.name}><b><PlayerName name={model.name}/></b> · {model.archetype || 'profile'} · {format(model.hands)} hands ({percent(model.confidence)} own evidence) · VPIP {percent(model.vpip)} · PFR {percent(model.pfr)} · 3-bet {percent(model.three_bet)} · fold to 3-bet {percent(model.fold_to_3bet)} · c-bet {percent(model.cbet)} · fold to c-bet {percent(model.fold_to_cbet)} · fold vs bet {(model.fold_vs_bet || []).map(v=>percent(v)).join(' / ')} · raise vs bet {(model.raise_vs_bet || []).map(v=>percent(v)).join(' / ')} · river bluff share {percent(model.river_bluff)}</p>)}<p className="footnote">These are the shrunk rates the decision priced each opponent with: their own counts blended with the population until evidence builds up.</p></details>}
    <div className="decision-stats"><span>RANGE EQUITY <b>{decision?.equity?.value == null ? 'Unavailable' : percent(decision.equity.value)}</b></span><span>POT ODDS <b>{decision?.pot_odds == null ? 'Unavailable' : percent(decision.pot_odds)}</b></span><span>SAMPLES <b>{decision?.equity?.samples == null ? 'Unavailable' : format(decision.equity.samples)}</b></span><span>ESTIMATE <b>{decision?.equity ? decision.equity.exact ? 'Exact · modeled range' : 'Monte Carlo' : 'Unavailable'}</b></span></div>
  </div>;
}

/** What a season-scoped figure covers, for the line under it. */
