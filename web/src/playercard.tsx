import { useEffect, useRef, useState } from 'react';
import { ArrowUpRight, X } from 'lucide-react';
import type { Bot, KeyHand, PlayerCard, Rates } from './types';
import { format } from './ui';
import { time } from './format';
import { PlayerName } from './playername';

/** The scout view (0296): one surface for everything we know about a player — identity, public
 * standing and reputation, the rates the decisions price them with beside the league, our record
 * against them as chip flow, the per-opponent corrections in force, and the hands we can replay.
 * Every count says what it counts and which hand base it comes from. It replaced the separate
 * Opponent intelligence table, Per-opponent reads list and opponent profile drawer, all of which
 * now open this one view through `openPlayerCard`. */

/** Open the scout view for `name` from anywhere (seats, the opponent list, the reads panel). */
export function openPlayerCard(name: string) {
  window.dispatchEvent(new CustomEvent('sv-open-player', {detail: name}));
}

const pct = (v: number) => `${Math.round(v * 100)}%`;
/** A share that must not round up to a claim it did not reach (confidence 0.9967 → 99.7%, not 100%). */
const conf = (v: number) => `${(v * 100).toFixed(1)}%`;
const num = (v: number | null | undefined, d = 0) => v == null ? '—' : v.toLocaleString('en-US', {maximumFractionDigits: d});
const signed = (v: number | null | undefined, d = 0) => v == null ? '—' : `${v > 0 ? '+' : ''}${num(v, d)}`;
const avg3 = (a: number[]) => (a[0] + a[1] + a[2]) / 3;

/** Six style axes, player against league, 0..1 each. */
function Radar({read, league}: {read: Rates; league: Rates}) {
  const axes: [string, (r: Rates) => number, number][] = [
    ['VPIP', r => r.vpip, 0.7], ['PFR', r => r.pfr, 0.5], ['3-BET', r => r.three_bet, 0.25],
    ['C-BET', r => r.cbet, 1], ['BET', r => avg3(r.bet_first), 0.8], ['FOLD', r => avg3(r.fold_vs_bet), 1],
  ];
  const cx = 110, cy = 105, R = 78;
  const at = (i: number, v: number) => { const a = -Math.PI / 2 + i * Math.PI / 3; return [cx + Math.cos(a) * R * v, cy + Math.sin(a) * R * v]; };
  const poly = (r: Rates) => axes.map(([, f, max], i) => at(i, Math.min(1, f(r) / max)).join(',')).join(' ');
  return <svg className="pc-radar" viewBox="0 0 220 210" role="img" aria-label="Style radar against the league average">
    {[0.33, 0.66, 1].map(k => <polygon key={k} points={axes.map((_, i) => at(i, k).join(',')).join(' ')} className="pc-radar-ring"/>)}
    {axes.map(([label], i) => { const [x, y] = at(i, 1.18); return <text key={label} x={x} y={y} className="pc-radar-label">{label}</text>; })}
    <polygon points={poly(league)} className="pc-radar-league"/>
    <polygon points={poly(read)} className="pc-radar-player"/>
  </svg>;
}

function StatBar({label, value, league, max = 1}: {label: string; value: number; league: number; max?: number}) {
  const w = (v: number) => `${Math.min(100, v / max * 100)}%`;
  const diff = value - league;
  return <div className="pc-stat" title={`${label}: ${pct(value)} · league ${pct(league)}`}>
    <span className="pc-stat-label">{label}</span>
    <span className="pc-stat-track"><i className="pc-stat-fill" style={{width: w(value)}}/><b className="pc-stat-league" style={{left: w(league)}}/></span>
    <span className="pc-stat-value">{pct(value)}<small className={Math.abs(diff) < 0.02 ? '' : diff > 0 ? 'up' : 'down'}>{Math.abs(diff) < 0.02 ? '=' : `${diff > 0 ? '▲' : '▼'}${Math.round(Math.abs(diff) * 100)}`}</small></span>
  </div>;
}

/** Cumulative chips over the shared hands, actual against all-in EV. This is a table result: it is
 * what our seat won while they sat in, shared with everyone else at the table (marked as such). */
function Trend({series}: {series: {hand:number;net:number;ev:number}[]}) {
  if (series.length < 2) return null;
  const vals = series.flatMap(p => [p.net, p.ev]);
  const lo = Math.min(0, ...vals), hi = Math.max(1, ...vals);
  const y = (v: number) => 70 - (v - lo) / (hi - lo) * 62;
  const line = (k: 'net' | 'ev') => series.map((p, i) => `${4 + i / (series.length - 1) * 292},${y(p[k])}`).join(' ');
  return <svg className="pc-trend" viewBox="0 0 300 76" role="img" aria-label="Our chips when this player was at the table">
    <line x1="4" x2="296" y1={y(0)} y2={y(0)} className="pc-zero"/>
    <polyline points={line('ev')} className="pc-trend-ev"/>
    <polyline points={line('net')} className="pc-trend-net"/>
  </svg>;
}

/** The hand base and confidence every rate on this view is read from, stated once. */
function BaseNote({card}: {card: PlayerCard}) {
  return <p className="pc-base">
    Read from <b>{num(card.hands_observed)} hands observed</b> — live tables plus imported history, older hands weighing less
    (the same base the opponent list shows) — shrunk toward the population at {conf(card.confidence)} confidence.
    Per-stat opportunity counts live in the Per-opponent reads panel.
  </p>;
}

/** Public standing and season history: the server's leaderboard and reputation store, as counts of
 * public-board hands, not of our observations. */
function Standing({card}: {card: PlayerCard}) {
  const s = card.leaderboard, rep = card.reputation;
  const finishes = rep?.finishes ?? [];
  const best = finishes.filter(f => f.rank === rep?.best_rank)[0];
  const otherNames = (rep?.names ?? []).filter(n => n !== card.name);
  return <>
    {(s || rep) && <section className="pc-standing" aria-label="Public standing and reputation">
      {s?.rank != null && <div><label>PUBLIC RANK</label><b>#{num(s.rank)}</b><small>{s.rank_delta ? `${s.rank_delta > 0 ? 'up' : 'down'} ${Math.abs(s.rank_delta)} since the last refresh` : 'unchanged since the last refresh'}</small></div>}
      {s?.score != null && <div><label>SEASON SCORE</label><b>{format(s.score)}</b><small>points on the public board{s.score_delta ? ` · ${signed(s.score_delta)} since the last refresh` : ''}</small></div>}
      {s?.hands != null && <div><label>SEASON HANDS</label><b>{format(s.hands)}</b><small>hands they played this season, public board{s.win_rate != null ? ` · ${pct(s.win_rate)} of them won` : ''}</small></div>}
      {rep?.lifetime_hands != null && <div><label>LIFETIME HANDS</label><b>{format(rep.lifetime_hands)}</b><small>across {num(rep.seasons)} recorded season{rep.seasons === 1 ? '' : 's'}</small></div>}
      {rep?.best_rank != null && <div><label>BEST SEASON RANK</label><b>#{num(rep.best_rank)}</b><small>{best ? `season ${best.season}, ${num(best.participants)} entrants` : 'their best finish on record'}</small></div>}
      {rep?.top10 != null && <div><label>TOP-10 FINISHES</label><b>{num(rep.top10)}</b><small>of {num(rep.seasons)} seasons{rep.strength != null ? ` · strength ${pct(rep.strength)}` : ''}</small></div>}
    </section>}
    {(otherNames.length > 0 || (rep?.strength != null && rep?.top10 == null)) && <p className="pc-facts">
      {otherNames.length > 0 && <>Also played as <b>{otherNames.join(', ')}</b> — one identity, all seasons merged. </>}
      {rep?.strength != null && rep?.top10 == null && <>Reputation strength {pct(rep.strength)} from their season finishes.</>}
    </p>}
  </>;
}

/** The hands we can open from this view: the newest shared hands the server sends (`vs_us.recent`,
 * newest first), or the two extremes when it sends none. A hand whose `bot` no live seat carries is
 * shown without a replay link rather than pointed at the wrong seat. */
function KeyHands({card, bots, onReplay}: {card: PlayerCard; bots: Bot[]; onReplay?: (slot: number, handId: string) => void}) {
  const picked = (card.vs_us.recent ?? [card.vs_us.biggest_win, card.vs_us.biggest_loss]);
  // A hand without an id cannot be replayed or named: it is left out rather than shown as a blank row.
  const rows = picked.filter((h): h is KeyHand => !!h && typeof h.hand_id === 'string' && h.hand_id.length > 0);
  if (!rows.length) return null;
  const slotOf = (bot: string) => bots.find(b => b.name === bot)?.slot;
  const when = (ts?: number) => ts ? time(ts, { seconds: true }) : 'time not recorded';
  return <section className="pc-recent" aria-label="Key hands with this player">
    <h3>{card.vs_us.recent ? 'RECENT HANDS' : 'BIGGEST HANDS'} <small>our net in the hand, chips; pot is the whole pot</small></h3>
    <ul>{rows.map(hand => {
      const slot = slotOf(hand.bot);
      return <li key={hand.hand_id}>
        <b className="mono" title={hand.hand_id}>#{hand.hand_id.slice(0, 8)}</b>
        <small>{when(hand.ts)} · {hand.bot}</small>
        <span className={hand.net < 0 ? 'negative' : 'positive'}>{signed(hand.net)}</span>
        <small>{format(hand.pot)} pot</small>
        {slot != null && onReplay
          ? <button type="button" className="text-button" onClick={() => onReplay(slot, hand.hand_id)}>Replay <ArrowUpRight size={11}/></button>
          : <span className="subtle" title="Recorded under an earlier bot name, which no live seat carries: the replay is filed under that seat's own slot.">no replay</span>}
      </li>;
    })}</ul>
  </section>;
}

/** Our own seat: its results and style next to the league, without the head-to-head parts. */
function OwnCard({card}: {card: PlayerCard}) {
  const v = card.vs_us;
  const edge = v.ev_bb100 ?? v.bb100;
  const rateBase = v.priced_hands == null ? `${num(v.hands)} hands` : `${num(v.priced_hands)} hands with recorded blinds`;
  return <>
    <section className="pc-scoreboard" aria-label="Our seat's results">
      <div className={`pc-big ${(edge ?? 0) < 0 ? 'negative' : 'positive'}`}><span>WIN RATE</span><b>{signed(edge, 0)}</b><small>bb/100 all-in EV{v.ev_confidence != null ? ` ± ${num(v.ev_confidence)}` : ''} over {rateBase}</small></div>
      <div className="pc-big" title="Pots this seat won, of the hands it played"><span>POTS WON</span><b>{num(v.won_pots)}</b><small>of {num(v.hands)} hands</small></div>
      <div className={`pc-big ${v.net < 0 ? 'negative' : 'positive'}`}><span>CHIPS WON</span><b>{signed(v.net)}</b><small>actual · EV {signed(v.ev_net)} over {num(v.hands)} hands</small></div>
      <div className="pc-form" aria-label="Last 10 hands, newest first"><span>FORM</span><div>{v.form.map((f, i) => <i key={i} className={`pc-pill ${f === 'W' ? 'win' : f === 'L' ? 'loss' : 'even'}`}>{f}</i>)}</div></div>
    </section>
    <div className="pc-grid">
      <section className="pc-panel"><h3>STYLE</h3><Radar read={card.read} league={card.league}/><p className="pc-legend"><i className="pc-key-player"/>{card.name}<i className="pc-key-league"/>League average</p></section>
      <section className="pc-panel"><h3>PREFLOP</h3>
        <StatBar label="VPIP" value={card.read.vpip} league={card.league.vpip}/>
        <StatBar label="PFR" value={card.read.pfr} league={card.league.pfr}/>
        <StatBar label="3-bet" value={card.read.three_bet} league={card.league.three_bet} max={0.3}/>
        <StatBar label="Fold to 3-bet" value={card.read.fold_to_3bet} league={card.league.fold_to_3bet}/>
        <StatBar label="Limp" value={card.read.limp} league={card.league.limp} max={0.5}/>
        <h3>POSTFLOP</h3>
        <StatBar label="C-bet" value={card.read.cbet} league={card.league.cbet}/>
        <StatBar label="Fold to c-bet" value={card.read.fold_to_cbet} league={card.league.fold_to_cbet}/>
        <StatBar label="Went to showdown" value={card.read.wtsd} league={card.league.wtsd}/>
        <StatBar label="Won at showdown" value={card.read.won_showdown} league={card.league.won_showdown}/>
        <StatBar label="River bluffs" value={card.read.river_bluff} league={card.league.river_bluff} max={0.6}/>
      </section>
      <section className="pc-panel"><h3>BY STREET</h3>
        <div className="pc-streets">{['Flop', 'Turn', 'River'].map((s, i) => <div key={s} className="pc-street"><b>{s}</b>
          <StatBar label="Bets" value={card.read.bet_first[i]} league={card.league.bet_first[i]}/>
          <StatBar label="Folds to bet" value={card.read.fold_vs_bet[i]} league={card.league.fold_vs_bet[i]}/>
          <StatBar label="Raises bet" value={card.read.raise_vs_bet[i]} league={card.league.raise_vs_bet[i]} max={0.4}/>
        </div>)}</div>
      </section>
      <section className="pc-panel"><h3>CHIPS WHEN THEY SAT IN</h3><Trend series={v.series}/><p className="pc-legend"><i className="pc-key-net"/>Actual<i className="pc-key-ev"/>All-in EV</p>
        <p className="pc-note">This is our seat's table result over the {num(v.hands)} shared hands, shared with everyone else in them; the head-to-head numbers above are the ones attributed to them.</p>
      </section>
    </div>
    <section className="pc-report"><h3>HOW THE TABLE SEES US</h3><p>{card.advice}</p><BaseNote card={card}/></section>
  </>;
}

export function PlayerCardHost({bots = [], onReplay}: {bots?: Bot[]; onReplay?: (slot: number, handId: string) => void}) {
  const [name, setName] = useState<string>();
  const [card, setCard] = useState<PlayerCard>();
  const [error, setError] = useState('');
  const closeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const open = (e: Event) => setName((e as CustomEvent<string>).detail);
    window.addEventListener('sv-open-player', open);
    return () => window.removeEventListener('sv-open-player', open);
  }, []);
  useEffect(() => {
    if (!name) return;
    let cancelled = false;
    setCard(undefined); setError('');
    fetch(`/api/players/${encodeURIComponent(name)}/card`).then(async r => {
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).detail || `Request failed (${r.status})`);
      return r.json();
    }).then(c => { if (!cancelled) setCard(c); }).catch(e => { if (!cancelled) setError((e as Error).message); });
    // Escape closes the topmost layer: a hand replay opened from this view keeps it.
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !document.querySelector('.replay-modal')) setName(undefined); };
    window.addEventListener('keydown', onKey);
    setTimeout(() => closeRef.current?.focus(), 0);
    return () => { cancelled = true; window.removeEventListener('keydown', onKey); };
  }, [name]);
  if (!name) return null;
  const v = card?.vs_us;
  const seat = card?.vs_seat;
  const fo = card?.corrections.fold_offset;
  const rr = card?.corrections.response_ratio;
  const st = card?.corrections.size_tell;
  return <div className="pc-backdrop" onClick={() => setName(undefined)}>
    <div className="pc-card" role="dialog" aria-modal="true" aria-label={`Scout view: ${name}`} onClick={e => e.stopPropagation()}>
      <div className="pc-stripe"/>
      <header className="pc-head">
        <div className="pc-avatar">{card?.avatar_url ? <img src={card.avatar_url} alt="" referrerPolicy="no-referrer"/> : <span>{name.slice(0, 2).toUpperCase()}</span>}</div>
        <div className="pc-title">
          <span className="pc-kicker">{card?.ours ? 'OUR SEAT' : 'SCOUTING REPORT'}</span>
          <h2>{name}</h2>
          <div className="pc-chips">{card && <span className="pc-style">{card.style}</span>}{card?.leaderboard?.rank != null && <span className="pc-rank">#{card.leaderboard.rank} · {num(card.leaderboard.score)} pts</span>}{card && <span className="pc-chip">{num(card.hands_observed)} hands observed</span>}{card && <span className="pc-chip" title="How much of their own rate the decisions use against them; the rest is shrunk to the population.">{conf(card.confidence)} confidence</span>}</div>
        </div>
        <button ref={closeRef} className="icon-button pc-close" aria-label="Close scout view" onClick={() => setName(undefined)}><X size={16}/></button>
      </header>
      {error && <p className="pc-error">{error}</p>}
      {!card && !error && <p className="pc-loading">Loading the file on {name}…</p>}
      {card && <Standing card={card}/>}
      {card && v && card.ours && <OwnCard card={card}/>}
      {card && v && !card.ours && <>
        <section className="pc-scoreboard" aria-label="Our record against this player">
          {seat
            ? <div className={`pc-big ${seat.bb_per_100 < 0 ? 'negative' : 'positive'}`} title="The chip flow attributed to their seat across every champion hand we shared — the experiment arms' treatment hands are out, as in the Rivals panel and the nemesis test (0361)"><span>OUR EDGE</span><b>{signed(seat.bb_per_100, 1)}</b><small>bb/100 vs their seat over {num(seat.hands)} attributed hands · 95% {signed(seat.low_95, 0)}..{signed(seat.high_95, 0)}</small></div>
            : <div className="pc-big" title="Fewer shared hands than the rivalry floor of 150"><span>OUR EDGE</span><b>—</b><small>too few shared hands</small></div>}
          <div className="pc-big" title="Pots we won with them dealt in, and pots they won from us"><span>WE WON – THEY WON</span><b>{num(v.won_pots)}<em>–</em>{num(v.lost_pots)}</b><small>pots with them in · theirs from us · {num(v.hands)} hands they were dealt into</small></div>
          <div className="pc-form" aria-label="Last 10 hands together, newest first"><span>FORM</span><div>{v.form.map((f, i) => <i key={i} className={`pc-pill ${f === 'W' ? 'win' : f === 'L' ? 'loss' : 'even'}`}>{f}</i>)}</div></div>
        </section>
        <BaseNote card={card}/>
        <KeyHands card={card} bots={bots} onReplay={onReplay}/>
        <div className="pc-grid">
          <section className="pc-panel"><h3>STYLE</h3><Radar read={card.read} league={card.league}/><p className="pc-legend"><i className="pc-key-player"/>{name}<i className="pc-key-league"/>League average</p></section>
          <section className="pc-panel"><h3>PREFLOP</h3>
            <StatBar label="VPIP" value={card.read.vpip} league={card.league.vpip}/>
            <StatBar label="PFR" value={card.read.pfr} league={card.league.pfr}/>
            <StatBar label="3-bet" value={card.read.three_bet} league={card.league.three_bet} max={0.3}/>
            <StatBar label="Fold to 3-bet" value={card.read.fold_to_3bet} league={card.league.fold_to_3bet}/>
            <StatBar label="Limp" value={card.read.limp} league={card.league.limp} max={0.5}/>
            <h3>POSTFLOP</h3>
            <StatBar label="C-bet" value={card.read.cbet} league={card.league.cbet}/>
            <StatBar label="Fold to c-bet" value={card.read.fold_to_cbet} league={card.league.fold_to_cbet}/>
            <StatBar label="Went to showdown" value={card.read.wtsd} league={card.league.wtsd}/>
            <StatBar label="Won at showdown" value={card.read.won_showdown} league={card.league.won_showdown}/>
            <StatBar label="River bluffs" value={card.read.river_bluff} league={card.league.river_bluff} max={0.6}/>
          </section>
          <section className="pc-panel"><h3>BY STREET</h3>
            <div className="pc-streets">{['Flop', 'Turn', 'River'].map((s, i) => <div key={s} className="pc-street"><b>{s}</b>
              <StatBar label="Bets" value={card.read.bet_first[i]} league={card.league.bet_first[i]}/>
              <StatBar label="Folds to bet" value={card.read.fold_vs_bet[i]} league={card.league.fold_vs_bet[i]}/>
              <StatBar label="Raises bet" value={card.read.raise_vs_bet[i]} league={card.league.raise_vs_bet[i]} max={0.4}/>
            </div>)}</div>
          </section>
          <section className="pc-panel"><h3>OUR RESULT TOGETHER</h3><Trend series={v.series}/><p className="pc-legend"><i className="pc-key-net"/>Actual<i className="pc-key-ev"/>All-in EV</p>
            {v.by_bot.length > 0 && <table className="pc-table"><tbody>{v.by_bot.map(b => <tr key={b.bot}><td><PlayerName name={b.bot}/></td><td>{num(b.hands)} hands</td><td className={b.net < 0 ? 'negative' : 'positive'}>{signed(b.net)}</td></tr>)}</tbody></table>}
            <p className="pc-note">Our seats as each hand recorded them: the fleet has flown under earlier names in past seasons. Chips when they sat in are shared with the table; the head-to-head read above is not.</p>
          </section>
        </div>
        <section className="pc-report"><h3>HOW WE PLAY THEM</h3><p>{card.advice}</p>
          <h4>CORRECTIONS IN FORCE</h4>
          <ul>{fo != null && <li>Heads-up they fold to our bets <b>{fo < 0 ? 'less' : 'more'}</b> than the model predicts (logit {signed(fo, 2)}); our bets are priced that way.</li>}
            {rr != null && <li>Against our bets they fold ×{rr[0].toFixed(2)}, call ×{rr[1].toFixed(2)} and raise ×{rr[2].toFixed(2)} as often as the response network expects.</li>}
            {st != null && <li>River sizing tell <b>{signed(st, 2)}</b>: their bet size tracks hand strength, so they are read as {st > 0 ? 'betting bigger with stronger hands' : 'betting bigger with weaker hands'} than the pool.</li>}
            {fo == null && rr == null && st == null && <li>No per-player correction yet: they are priced from their stats until enough bets against them are scored.</li>}</ul>
          <p className="pc-note">Each correction goes live only while it predicts newer hands better than the shared model (Per-opponent reads has the evidence).</p>
        </section>
      </>}
    </div>
  </div>;
}
