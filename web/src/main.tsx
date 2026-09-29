import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource/inter/400.css';
import '@fontsource/inter/500.css';
import '@fontsource/inter/600.css';
import '@fontsource/barlow-condensed/600.css';
import '@fontsource/barlow-condensed/700.css';
import '@fontsource/ibm-plex-mono/400.css';
import './style.css';
import { Activity, ArrowUpCircle, ArrowUpRight, Crown, Gauge, Sparkles, Swords, ChevronRight, Cpu, FlaskConical, Grip, History, Layers, LayoutGrid, ListFilter, Radio, RotateCcw, Search, ShieldCheck, Spade, Terminal, TrendingUp, Users, WifiOff, X, Crosshair } from 'lucide-react';
import { HelpPage } from './help';
import { DocsPage } from './docs';
import { SetupPage } from './setup';
import { ResultsMonitor } from './monitor';
import { UpdatesPanel } from './updates';
import { HostPanel } from './host';
import { BotControls, DashboardHeader, DecisionTelemetry } from './dashboard';
import { LeakFinder, RangeExplorer } from './lab';
import { WidgetBoard, ViewTabs, loadView } from './widgets';
import { PlayerCardHost, openPlayerCard } from './playercard';
import { registerNames, SELECT_BOT_EVENT } from './playername';
import { AccuracyPanel, QuizPage, WiringPanel } from './games';
import { ActionTicker, BadgeRace, CalibrationPanel, FleetRace, HighlightsPanel, LivePulse, RivalsPanel, SeasonRace, StoriesPanel, WinToasts } from './fun';
import type { Bot, Hand, Opponent, ReplayEvent, Snapshot, TableBot } from './types';
import { format, signed, percent, suitMap, dotClass, ThemeToggle, api, Card, Panel, Empty } from './ui';
import { time, TURN_DEADLINE_S } from './format';
import type { TableTheme } from './ui';
import { announceSession, StaleNote, usePoll } from './api';
import { PokerTable, TvMode, DecisionStrip } from './table';
import { scopeLabel, Performance, Autonomy, ExperimentCard, ExperimentModePanel, Season, Profile, SearchFunnel } from './training';
import { Replay, StartingHands } from './panels';
import { IntelPanel } from './intel';
import { TimelinePanel } from './timeline';
import { readLocal } from './storage';

/** openpoker.ai's per-table route is `/arena/<table id>` (operator, 2026-09-27). It supersedes 0298,
 *  which probed `/table/<id>` and `/tables` and concluded no per-table route existed: neither is the
 *  route the site actually serves. */
const ARENA_URL = 'https://openpoker.ai/arena';

/** What the bot is doing right now, from the server's own status (`state.rs` builds
 *  `playing at table 50ea6d9a`, the id truncated to 8 chars for display), linked to that table's
 *  arena page. The label stays the short id; the href needs the full uuid, which the status string
 *  does not carry, so it comes from `bot.table_id` — the untruncated one the API sends. */
function statusPhrase(bot?: Bot) {
  const status = bot?.status ?? '';
  const at = / at table (\S+)$/.exec(status);
  if (!bot?.connected || !at) return status || 'idle';
  const href = bot.table_id ? `${ARENA_URL}/${bot.table_id}` : ARENA_URL;
  return <>{status.slice(0, at.index)} at table <a className="table-link" href={href} target="_blank" rel="noreferrer" title="Watch this table on openpoker.ai">{at[1]}</a></>;
}

function App() {
  const [helpRoute,setHelpRoute] = useState(location.hash);
  useEffect(() => { const update = () => setHelpRoute(location.hash); window.addEventListener("hashchange", update); return () => window.removeEventListener("hashchange", update); }, []);
  useEffect(() => { if (helpRoute.startsWith("#help/")) document.getElementById(helpRoute.slice(6))?.scrollIntoView(); else if (helpRoute === "#help") window.scrollTo(0, 0); }, [helpRoute]);
  const [snapshot,setSnapshot] = useState<Snapshot>();
  // null until the health probe below answers: which listener this page was served by decides
  // whether there is an operator session at all (`SVANBOT_TV_PORT` vs `SVANBOT_WEB_PORT`).
  const [publicTv,setPublicTv] = useState<boolean | null>(null);
  const [tvBots,setTvBots] = useState<TableBot[]>([]);
  const [selected,setSelected] = useState(Number(readLocal('svan-slot') ?? '-1'));
  const [connected,setConnected] = useState(false);
  const [error,setError] = useState('');
  const [loginRequired,setLoginRequired] = useState(false);
  const [operatorToken,setOperatorToken] = useState('');
  const [loginAttempt,setLoginAttempt] = useState(0);
  const [busy,setBusy] = useState(false);
  const [query,setQuery] = useState('');
  const [logQuery,setLogQuery] = useState('');
  const [settings,setSettings] = useState(false);
  const [watchAll,setWatchAll] = useState(false);
  const [replay,setReplay] = useState<{hand:Hand;events:ReplayEvent[]}>();
  const [compact,setCompact] = useState(readLocal('svan-compact') === 'true');
  const [tableTheme,setTableTheme] = useState<TableTheme>(() => {
    const saved = readLocal('svan-table-theme');
    return saved === 'felt' || saved === 'midnight' ? saved : 'arena';
  });
  const changeTheme = (theme: TableTheme) => { setTableTheme(theme); localStorage.setItem('svan-table-theme', theme); };
  const [arranging,setArranging] = useState(false);
  const [view,setView] = useState(loadView);
  const eventRef = useRef<EventSource | null>(null);
  const bot = snapshot?.bots.find(candidate => candidate.slot === selected) || snapshot?.bots[0];
  const handsPoll = usePoll<Hand[]>(bot ? `/bots/${bot.slot}/hands` : null, 30000);
  const opponentsPoll = usePoll<Opponent[]>(bot ? `/bots/${bot.slot}/opponents` : null, 30000);
  const hands = handsPoll.data ?? [];
  const opponents = opponentsPoll.data ?? [];
  // Every name on the page is clickable (0297): ours switch the dashboard to that bot, others open their scout view.
  registerNames((snapshot?.bots || []).map(b => b.name), opponents.map(o => o.name));
  const botsRef = useRef(snapshot?.bots || []);
  botsRef.current = snapshot?.bots || [];
  useEffect(() => {
    const select = (e: Event) => {
      const target = botsRef.current.find(b => b.name === (e as CustomEvent<string>).detail);
      if (!target) return;
      setSelected(target.slot);
      setWatchAll(false);
      try { localStorage.setItem('svan-slot', String(target.slot)); } catch { /* storage unavailable */ }
      window.scrollTo({ top: 0, behavior: 'smooth' });
    };
    window.addEventListener(SELECT_BOT_EVENT, select);
    return () => window.removeEventListener(SELECT_BOT_EVENT, select);
  }, []);
  const activeCount = snapshot?.bots.filter(candidate => candidate.connected).length || 0;
  /** Seats online, or that nothing has answered yet (0298: the line never invents a fraction). */
  const online = snapshot ? `${activeCount} of ${snapshot.bots.length} bots online` : 'waiting for the server';

  useEffect(() => {
    let cancelled = false;
    const initialize = async () => {
      try {
        // Which listener served this page? The public TV answers its own health with `public: true`
        // and has no session route at all; the dashboard answers false. Asked first, so a spectator
        // is never shown a login form that cannot be satisfied, and never posts a token to an
        // endpoint that is not there.
        const listener = await api<{public?: boolean}>('/health').catch(() => undefined);
        if (cancelled) return;
        setPublicTv(listener?.public === true);
        if (listener?.public === true) return;
        await api('/session', {token: operatorToken});
        announceSession();
        setLoginRequired(false);
        setOperatorToken('');
        setError('');
        const initial = await api<Snapshot>('/state');
        if(cancelled) return;
        setSnapshot(initial);
        const source = new EventSource('/api/events');
        eventRef.current = source;
        source.addEventListener('state', event => {setSnapshot(JSON.parse((event as MessageEvent).data));setConnected(true);});
        // A bot's live table the moment it changes (0211); metrics stay from the last snapshot.
        source.addEventListener('table', event => {
          const {slot, bot: live} = JSON.parse((event as MessageEvent).data) as {slot: number; bot: Omit<Bot, 'metrics'>};
          setSnapshot(prev => prev && {...prev, bots: prev.bots.map(b => b.slot === slot ? {...b, ...live, slot: b.slot, metrics: b.metrics} : b)});
          setConnected(true);
        });
        for (const kind of ['action', 'board', 'result', 'decision', 'hand']) {
          source.addEventListener(kind, event => window.dispatchEvent(new CustomEvent('sv-live', {detail: JSON.parse((event as MessageEvent).data)})));
        }
        source.onerror = () => setConnected(false);
        source.onopen = () => setConnected(true);
      } catch(failure) {if(!cancelled) {setError((failure as Error).message);setLoginRequired(true);}}
    };
    void initialize();
    return () => {cancelled = true;eventRef.current?.close();};
  }, [loginAttempt]);

  // The public TV's whole feed: its table payload and its stream, the only two routes that listener
  // serves. The browser reconnects a dropped EventSource on its own, and a table that stops moving
  // is the staleness signal — there is no operator here to read a banner.
  useEffect(() => {
    if(publicTv !== true) return;
    const upsert = (slot: number, next: TableBot) => setTvBots(previous => previous.some(b => b.slot === slot) ? previous.map(b => b.slot === slot ? next : b) : [...previous, next].sort((a,b) => a.slot - b.slot));
    void api<{bots: TableBot[]}>('/tv').then(initial => initial.bots.forEach(b => upsert(b.slot, b))).catch(() => undefined);
    const source = new EventSource('/api/tv/events');
    source.addEventListener('table', event => {
      const {slot, bot: live} = JSON.parse((event as MessageEvent).data) as {slot: number; bot: TableBot};
      upsert(slot, live);
    });
    return () => source.close();
  }, [publicTv]);

  useEffect(() => {
    if(!bot) return;
    // Refresh when this bot finishes a hand (0211); the slow poll only covers a missed event.
    const onHand = (event: Event) => { const e = (event as CustomEvent<{type: string; slot: number}>).detail; if (e.type === 'hand' && e.slot === bot.slot) { handsPoll.refresh(); opponentsPoll.refresh(); } };
    window.addEventListener('sv-live', onHand);
    return () => window.removeEventListener('sv-live', onHand);
  }, [bot?.slot]);

  async function command(action: string) {
    if(!bot) return;
    setBusy(true);
    try {await api(`/bots/${bot.slot}/command`,{command:action});setError('');setSnapshot(await api<Snapshot>('/state'));}
    catch(failure) {setError((failure as Error).message);} finally {setBusy(false);}
  }
  async function trainingCommand(action:string, extra:object = {}) {
    setBusy(true);
    try {await api('/training/command',{command:action,...extra});setError('');setSnapshot(await api<Snapshot>('/state'));}
    catch(failure) {setError((failure as Error).message);} finally {setBusy(false);}
  }
  async function openReplayFor(slot:number, handId:string) {
    try {
      const events = await api<ReplayEvent[]>(`/bots/${slot}/hands/${encodeURIComponent(handId)}`);
      const start = events.find(e => e.type === 'hand_start');
      const end = events.find(e => e.type === 'hand_result');
      const hand: Hand = {id: 0, hand_id: handId, ts: start?.ts || 0, hole: (start?.data.hole_cards as string[]) || [], board: (end?.data.board as string[]) || [], net: (end?.data.net as number) ?? null, big_blind: 20, version: ''};
      setReplay({hand, events});
    } catch(failure) {setError((failure as Error).message);}
  }
  async function openReplay(hand:Hand) {
    if(!bot) return;
    try {setReplay({hand,events:await api<ReplayEvent[]>(`/bots/${bot.slot}/hands/${encodeURIComponent(hand.hand_id)}`)});}
    catch(failure) {setError((failure as Error).message);}
  }
  const filteredHands = hands.filter(hand => `${hand.hand_id} ${hand.hole.join(' ')} ${hand.board.join(' ')}`.toLowerCase().includes(query.toLowerCase()));
  const hero = bot?.seats.find(seat => seat.seat === bot.hero_seat);
  // Until `/health` answers, nothing is rendered. Every panel below starts its timer when it mounts,
  // so mounting them first — which is what `null` used to mean — had the public page ask its listener
  // for six dashboard routes it does not serve, all of them 404s, before the answer that would have
  // said "there are no panels here" arrived (issue #335). The shell holds that one round trip; a
  // health probe that fails answers `undefined`, which is the dashboard, so this cannot be a dead end.
  if(publicTv === null) return <div className="app boot"><span className="status-dot connecting"/>Connecting…</div>;
  // The public TV (`SVANBOT_TV_PORT`) renders the table view and nothing else: no header, no panels,
  // no login form, no hash routes. What a spectator can reach is what that listener serves.
  if(publicTv) return <div className="app"><TvMode bots={tvBots} theme={tableTheme} public/></div>;
  return <div className={`app ${compact ? 'compact' : ''} `}>
    <WinToasts bots={snapshot?.bots || []}/>
    <PlayerCardHost bots={snapshot?.bots || []} onReplay={openReplayFor}/>
    <DashboardHeader bots={snapshot?.bots || []} selectedSlot={bot?.slot} connected={connected} onSelect={slot=>{setSelected(slot);localStorage.setItem('svan-slot',String(slot));}} onSettings={()=>setSettings(true)}/>
    <div className="workspace-header"><div className="breadcrumb"><span>WORKSPACE</span><ChevronRight size={12}/><b>{watchAll ? 'Fleet overview' : bot?.name || 'Control room'}</b><span className="environment-tag">OPENPOKER · VIRTUAL CHIPS</span></div><div className="workspace-tools"><button className={watchAll ? 'text-button active' : 'text-button'} onClick={()=>setWatchAll(!watchAll)}><LayoutGrid size={13}/>{watchAll ? 'Focus table' : 'Watch all'}</button><button className={arranging ? 'text-button active' : 'text-button'} aria-pressed={arranging} onClick={()=>setArranging(!arranging)}><Grip size={13}/>{arranging ? 'Arranging…' : 'Arrange widgets'}</button><span className="updated">{snapshot ? `Updated ${time(snapshot.updated, { seconds: true })}` : 'Waiting for server'}</span></div></div>
    {loginRequired && <form className="error-banner" onSubmit={event=>{event.preventDefault();setLoginAttempt(value=>value+1);}}><label>Operator token <input type="password" autoComplete="off" aria-label="Operator token" value={operatorToken} onChange={event=>setOperatorToken(event.target.value)}/></label><button className="button" type="submit">Unlock control room</button><span>Use SVANBOT_WEB__OPERATOR_TOKEN from .env. Plain HTTP is supported; keep the token private.</span></form>}
    {error && <div className="error-banner" role="alert"><WifiOff size={15}/>{error}<button aria-label="Dismiss error" onClick={()=>setError('')}><X size={16}/></button></div>}
    {!connected && snapshot && <div className="connection-banner"><Radio size={14}/>Connection interrupted. Showing the last received state; controls reconnect automatically.</div>}
    {helpRoute.startsWith("#help") && <HelpPage/>}
    {helpRoute.startsWith("#docs") && <DocsPage/>}
    {helpRoute.startsWith("#tv") && <TvMode bots={snapshot?.bots || []} theme={tableTheme}/>}
    {helpRoute.startsWith("#quiz") && <QuizPage/>}
    {helpRoute.startsWith("#setup") && <SetupPage/>}
    <main hidden={helpRoute.startsWith("#help") || helpRoute.startsWith("#docs") || helpRoute.startsWith("#tv") || helpRoute.startsWith("#quiz") || helpRoute.startsWith("#setup")}><div className="overview"><div className="overview-title"><p><span className={`status-dot ${dotClass(bot?.mode)}`}/><span className="status-line">{bot?.name ? <><b>{bot.name}</b> · {bot.connected ? statusPhrase(bot) : bot.mode === 'connecting' ? 'connecting' : bot.mode === 'paused' ? 'paused' : 'offline'} · {online}</> : <>Ready to connect · {online}</>}</span></p></div><div className="overview-stats"><div><label>TABLE STACK</label><b>{format(hero?.stack)}</b><small>{hero ? `${format(hero.stack / (bot?.big_blind || 20),1)} BB effective balance` : 'No active seat'}</small></div><div><label>RECORDED NET WINNINGS</label><b className={(bot?.metrics.net_chips || 0) < 0 ? 'negative' : 'positive'}>{signed(bot?.metrics.net_chips)}</b><small>{scopeLabel(bot?.metrics.season)} · recorded hands only{bot?.metrics.all_time ? ` · lifetime ${signed(bot.metrics.all_time.net_chips)}` : ''}</small></div><div><label>HANDS PLAYED</label><b>{format(bot?.metrics.hands || 0)}<span className="tiny-label">HANDS</span></b><small>{scopeLabel(bot?.metrics.season)}{bot?.metrics.p95_ms == null ? '' : ` · ${format(bot.metrics.p95_ms)} ms decision p95`}</small></div></div>{bot?.metrics.error && <p className="stale-note" role="status">Store read failed ({bot.metrics.error}) — the figures above are the last good reading, not zeros.</p>}</div>
      <DecisionTelemetry bot={bot} training={snapshot?.training}/>
      {watchAll && <div className="fleet-grid" aria-label="Live fleet tables">{snapshot?.bots.map(candidate=><section className="fleet-table" key={candidate.slot}><button className="fleet-heading" onClick={()=>{setSelected(candidate.slot);setWatchAll(false);}}><span className={`status-dot ${dotClass(candidate.mode)}`}/><b>{candidate.name}</b><span>{candidate.connected ? candidate.street : candidate.status}</span><ChevronRight size={14}/></button><PokerTable bot={candidate} theme={tableTheme}/><div className="fleet-footer"><span>{candidate.table_id || 'Waiting for table'}</span>{candidate.metrics.error && <span className="footnote amber" title={candidate.metrics.error}>last good reading — store read failed</span>}<b className={candidate.metrics.net_chips < 0 ? 'negative':'positive'}>{signed(candidate.metrics.net_chips)} chips</b></div></section>)}</div>}
      <ViewTabs view={view} onChange={setView}/>
      {view === 'results' && <TimelinePanel onReplay={openReplayFor}/>}
      <WidgetBoard view={view} editing={arranging} onDoneEditing={()=>setArranging(false)} defaults={{left:['autonomy', 'experiments', 'experiment-mode', 'calibration', 'accuracy', 'wiring', 'highlights', 'health', 'privacy'], center:['table', 'ranges', 'ticker', 'leaks', 'fleet-race', 'starting-hands', 'recent-hands', 'opponents'], right:['monitor', 'updates', 'host', 'season-race', 'badges', 'rivals', 'intel', 'stories', 'performance', 'champion', 'season', 'activity'], hidden:[]}} widgets={[
    {id:'autonomy', title:'Autonomy', node:<Panel title="Autonomy" icon={<Cpu size={15}/>} aside={<span className="tag amber">{snapshot?.training.automatic ? 'AUTOPILOT' : 'MANUAL'}</span>}><Autonomy training={snapshot?.training} onCommand={trainingCommand} busy={busy}/></Panel>},
    {id:'experiments', title:'Experiments', node:<Panel title="Experiments" icon={<FlaskConical size={15}/>} aside={<span className="count">{snapshot?.training.experiments.length || 0}</span>}>{snapshot?.training.search_funnel?.total ? <SearchFunnel funnel={snapshot.training.search_funnel}/> : null}{snapshot?.training.experiments.length ? <div className="experiments">{snapshot.training.experiments.slice(0,6).map(experiment=><ExperimentCard experiment={experiment} key={experiment.id}/>)}</div> : <Empty title="The next edge is waiting" detail="Training cycles compare challengers against your current champion." icon={<FlaskConical size={24}/>}/>}</Panel>},
    {id:'experiment-mode', title:'Experiment mode', node:<Panel title="Experiment mode" icon={<FlaskConical size={15}/>} aside={<span className="tag">TOP FOUR</span>}><ExperimentModePanel/></Panel>},
    {id:'calibration', title:'Self-calibration', node:<Panel title="Self-calibration" icon={<Gauge size={15}/>} aside={<span className="tag">AUTO</span>}><CalibrationPanel/></Panel>},
    {id:'accuracy', title:'Decision accuracy', node:<Panel title="Decision accuracy" icon={<Gauge size={15}/>} aside={<span className="tag">7 DAYS</span>}><AccuracyPanel/></Panel>},
    {id:'wiring', title:'Wiring table', node:<Panel title="Wiring table" icon={<Layers size={15}/>} aside={<span className="tag">MEASURED</span>}><WiringPanel/></Panel>},
    {id:'highlights', title:'Highlights', node:<Panel title="Highlights" icon={<Sparkles size={15}/>}><HighlightsPanel onReplay={openReplayFor}/></Panel>},
    {id:'health', title:'Runtime health', node:<Panel title="Runtime health" icon={<ShieldCheck size={15}/>}><div className="health-list"><div><span>Transport</span><b className={bot?.connected ? 'positive':'muted'}>{bot?.connected ? 'Connected':'Offline'}</b></div><div><span>Decision latency · p95</span><b>{bot?.metrics.p95_ms == null ? 'Unavailable' : `${format(bot.metrics.p95_ms)} ms`}</b></div><div><span>Rejected actions</span><b className={bot?.metrics.rejected ? 'negative':'positive'}>{format(bot?.metrics.rejected || 0)}</b></div><div><span>State hash · verified / mismatched</span><b className={bot?.metrics.state_hash?.bad ? 'negative':'positive'}>{format(bot?.metrics.state_hash?.ok || 0)} / {format(bot?.metrics.state_hash?.bad || 0)}</b></div><div><span>Recovery</span><b>Automatic</b></div><div><span>Hands awaiting a store retry</span><b className={snapshot?.ops?.unstored_hands ? 'negative':'positive'}>{format(snapshot?.ops?.unstored_hands || 0)}</b></div><div><span>Keepalive</span><b className="nowrap" title={snapshot?.ops?.keepalive_last || 'The 5-minute keepalive has never had to restart the fleet.'}>{snapshot?.ops?.hold_until === 'forever' ? 'held until started' : typeof snapshot?.ops?.hold_until === 'number' ? `held until ${time(snapshot.ops.hold_until)}` : snapshot?.ops?.keepalive_last ? `last acted ${time(snapshot.ops.keepalive_last)}` : 'never needed'}</b></div></div><div className="health-note"><ShieldCheck size={17}/><span>Legal fallback prepared before every strategy calculation.</span></div></Panel>},
    {id:'privacy', title:'Self-hosted note', node:<div className="local-note"><span className="status-dot"/><span>SELF-HOSTED & PRIVATE<br/><small>Keys stay on your machine.</small></span></div>},
    {id:'table', title:'Live table', node:<Panel title="Live table" icon={<Spade size={15}/>} className="table-panel" aside={<div className="table-heading-tags"><span className="tag">{bot?.turn && bot.turn_started ? `${Math.max(0,Math.ceil(TURN_DEADLINE_S-(Date.now()/1000-bot.turn_started)))}s TO ACT` : bot?.street || 'STANDBY'}</span><span className="table-id">{bot?.table_id ? `#${bot.table_id.slice(0,12)}` : 'NO ACTIVE TABLE'}</span></div>}><div className="table-toolbar"><span><span className={`status-dot ${dotClass(bot?.mode)}`}/>{bot?.mode === 'playing' ? 'Live feed' : bot?.mode === 'connecting' ? 'Connecting…' : bot?.mode === 'paused' ? 'Paused' : 'Awaiting connection'}</span><ThemeToggle theme={tableTheme} onChange={changeTheme}/><BotControls bot={bot} busy={busy} connected={connected} onCommand={command}/></div><PokerTable bot={bot} theme={tableTheme}/><LivePulse bots={snapshot?.bots || []}/><DecisionStrip decision={bot?.decision} bot={bot}/></Panel>},
    {id:'ranges', title:'Range explorer', node:<Panel title="Range explorer" icon={<Layers size={15}/>} aside={<span className="tag">LAST DECISION</span>}><RangeExplorer slot={bot?.slot} decisionKey={`${bot?.decision?.action}-${bot?.hand_id}-${bot?.street}`}/></Panel>},
    {id:'ticker', title:'Live action', node:<Panel title="Live action" icon={<Radio size={15}/>} aside={<span className="tag">REALTIME</span>}><ActionTicker selectedSlot={bot?.slot}/></Panel>},
    {id:'monitor', title:'Results monitor', node:<Panel title="Results monitor" icon={<Activity size={15}/>} aside={<span className="live-label"><span className="status-dot"/>30 S</span>}><ResultsMonitor/></Panel>},
    {id:'updates', title:'Releases & updates', node:<Panel title="Releases & updates" icon={<ArrowUpCircle size={15}/>} aside={<span className="tag">SELF-HOSTED</span>}><UpdatesPanel/></Panel>},
    {id:'host', title:'Host check', node:<Panel title="Host check" icon={<Cpu size={15}/>} aside={<span className="tag">READ-ONLY</span>}><HostPanel/></Panel>},
    {id:'leaks', title:'Leak finder', node:<Panel title="Leak finder" icon={<Search size={15}/>} aside={<span className="tag">ALL HANDS</span>}><LeakFinder/></Panel>},
    {id:'fleet-race', title:'Fleet race', node:<Panel title="Fleet race" icon={<Swords size={15}/>} aside={<span className="tag">ALL BOTS</span>}><FleetRace/></Panel>},
    {id:'starting-hands', title:'Starting hand library', node:<StartingHands version={snapshot?.training.champion.version}/>},
    {id:'recent-hands', title:'Recent hands', node:<Panel title="Recent hands" icon={<History size={15}/>} aside={<span className="subtle">Click a hand to replay <ArrowUpRight size={11}/></span>}><StaleNote poll={handsPoll}/><div className="table-filter"><label><Search size={13}/><input aria-label="Search hands" placeholder="Search hands, cards, boards…" value={query} onChange={event=>setQuery(event.target.value)}/></label><span>{filteredHands.length} hands <ListFilter size={13}/></span></div><div className="data-table-wrap"><table className="data-table hands-table"><thead><tr><th>HAND / TIME</th><th>HOLE CARDS</th><th>BOARD</th><th>NET</th><th/></tr></thead><tbody>{filteredHands.slice(0,8).map(hand=><tr key={hand.id} onClick={()=>openReplay(hand)} tabIndex={0} onKeyDown={event=>{if(event.key==='Enter') void openReplay(hand);}}><td><b title={hand.hand_id}>#{hand.hand_id.slice(0,9).replace(/-$/,'')}</b><small>{time(hand.ts, { seconds: true })}</small></td><td><div className="inline-cards">{hand.hole.map((card,index)=><Card key={index} small card={card}/>)}</div></td><td><div className="board-text">{hand.board.map((card,index)=><span key={index} className={'hd'.includes(card[1])?'card-red':''}>{card[0]}{suitMap[card[1]]}</span>)}</div></td><td className={hand.net != null && hand.net < 0 ? 'negative':'positive'}>{signed(hand.net)}</td><td><ChevronRight size={14}/></td></tr>)}</tbody></table></div>{!filteredHands.length && !handsPoll.error && <Empty title={query ? 'No matching hands' : 'Every hand tells a story'} detail={query ? 'Try another card or hand ID.' : 'Completed hands appear here with full action-by-action replay.'} icon={<History size={22}/>}/>}</Panel>},
    {id:'opponents', title:'Opponent intelligence', node:<Panel title="Opponent intelligence" icon={<Users size={15}/>} aside={<span className="tag">{opponents.length} PROFILES</span>}><StaleNote poll={opponentsPoll}/><p className="footnote">Every player we have observed, most-seen first. Hands are counted from live tables plus downloaded history, older hands weighing less. Click a player for their scout view.</p><ul className="opponent-list">{opponents.slice(0,20).map(opponent=><li key={opponent.name}><button type="button" className="opponent-row" onClick={()=>openPlayerCard(opponent.name)}><span className="opponent-avatar" aria-hidden="true">{opponent.name.slice(0,1)}</span><span className="opponent-row-who"><b>{opponent.name}</b><small title={opponent.advice}>{opponent.style}</small></span><span className="opponent-row-stats">{(['vpip','pfr','aggression','fold_to_bet'] as const).map(metric=>{const est=opponent[metric]; const samples=est?.samples ?? est?.count; const hasInterval=est?.lower != null && est?.upper != null; const label=metric==='fold_to_bet'?'Folds to a bet':metric[0].toUpperCase()+metric.slice(1); return <span key={metric} title={est ? `${label}: ${percent(est.value)} over ${format(samples)} opportunities${hasInterval ? `; 95% interval ${percent(est.lower)}–${percent(est.upper)}` : ''}`:`No ${label} observations yet`}><small>{label}</small><b>{percent(est?.value)}</b></span>;})}</span><span className="opponent-row-hands"><b>{format(opponent.evidence_hands)}</b><small>hands observed</small></span><ChevronRight size={14} aria-hidden="true"/></button></li>)}</ul>{!opponents.length && !opponentsPoll.error && <Empty title="Observe. Understand. Adapt." detail="Opponent profiles build from public actions, with sample counts and uncertainty." icon={<Users size={23}/>}/>}</Panel>},
    {id:'season-race', title:'Season race', node:<Panel title="Season race" icon={<Crown size={15}/>} aside={<span className="tag">RANK TRACKING</span>}><SeasonRace/></Panel>},
    {id:'badges', title:'Badge race', node:<Panel title="Badge race" icon={<Crown size={15}/>} aside={<span className="tag">SEASON END</span>}><BadgeRace/></Panel>},
    {id:'stories', title:'Stories', node:<Panel title="Stories" icon={<Sparkles size={15}/>} aside={<span className="tag">RECAP</span>}><StoriesPanel/></Panel>},
    {id:'rivals', title:'Rivals', node:<Panel title="Rivals" icon={<Swords size={15}/>} aside={<span className="tag">CHAMPION HANDS</span>}><RivalsPanel/></Panel>},
    {id:'intel', title:'Per-opponent reads', node:<Panel title="Per-opponent reads" icon={<Crosshair size={15}/>} aside={<span className="tag">PER OPPONENT</span>}><IntelPanel/></Panel>},
    {id:'performance', title:'Performance', node:<Panel title="Performance" icon={<TrendingUp size={15}/>} aside={<span className="tag">{scopeLabel(bot?.metrics.season).toUpperCase()}</span>}><Performance metrics={bot?.metrics}/></Panel>},
    {id:'champion', title:'Champion profile', node:<Panel title="Champion profile" icon={<Layers size={15}/>}><Profile training={snapshot?.training} bot={bot}/>{snapshot?.training.can_rollback && <button className="button rollback" disabled={busy} onClick={()=>trainingCommand('rollback')}><RotateCcw size={12}/>Restore previous champion</button>}</Panel>},
    {id:'season', title:'Season ledger', node:<Season bot={bot}/>},
    {id:'activity', title:'Activity log', node:<Panel title="Activity log" icon={<Terminal size={15}/>} aside={<span className="live-label"><span className="status-dot"/>LIVE</span>}><label className="log-search"><Search size={12}/><input aria-label="Search activity" placeholder="Filter activity…" value={logQuery} onChange={event=>setLogQuery(event.target.value)}/></label><div className="log-list">{snapshot?.logs.filter(log=>(log.slot === bot?.slot || log.slot === null) && log.message.toLowerCase().includes(logQuery.toLowerCase())).slice(0,15).map(log=><div className={`log-entry ${log.level}`} key={log.id}><time>{time(log.ts, { seconds: true })}</time><p>{log.message}</p></div>)}{!snapshot?.logs.length && <div className="log-placeholder"><span>&gt;_</span>System ready.<br/>Waiting for your first session.</div>}</div></Panel>},
      ]}/>
      <footer><span><Spade size={12}/> SVANBOT <span className="footer-separator">/</span> Built on proven control-room foundations.</span><span>LOCAL CONTROL ROOM</span></footer>
    </main>
    {settings && <div className="modal-backdrop" onClick={()=>setSettings(false)}><section className="modal" role="dialog" aria-modal="true" aria-label="Settings" onClick={event=>event.stopPropagation()}><div className="modal-heading"><div><span className="eyebrow">YOUR WORKSPACE</span><h2>Control room settings</h2></div><button className="icon-button" aria-label="Close settings" onClick={()=>setSettings(false)}><X/></button></div><div className="settings-row"><div><strong>Compact layout</strong><p>Fit more information on your screen.</p></div><button className={`toggle ${compact ? 'on':''}`} role="switch" aria-checked={compact} aria-label="Compact layout" onClick={()=>{setCompact(!compact);localStorage.setItem('svan-compact',String(!compact));}}><i/></button></div><div className="settings-row"><div><strong>Automatic training</strong><p>Evaluate challengers automatically, even while tables are stopped.</p></div><button className={`toggle ${snapshot?.training.automatic ? 'on':''}`} role="switch" aria-checked={!!snapshot?.training.automatic} aria-label="Automatic training" onClick={()=>trainingCommand('automatic',{enabled:!snapshot?.training.automatic})}><i/></button></div><div className="settings-details"><div><span>Configured bots</span><b>{snapshot?.config.configured_slots}</b></div><div><span>Buy-in</span><b>{format(snapshot?.config.buy_in)} chips</b></div><div><span>Automatic rebuy</span><b>{snapshot?.config.auto_rebuy ? 'Enabled':'Disabled'}</b></div><div><span>Credentials</span><b>Server-side .env</b></div></div><div className="settings-row"><div><strong>Bot setup</strong><p>Add or remove bots, paste API keys, switch bots off, set the buy-in.</p></div><a className="button" href="#setup" onClick={()=>setSettings(false)}>Open bot setup</a></div><p className="footnote">Pause finishes the current hand before leaving. Stop requests an immediate departure. Strategy promotions take effect on the next hand.</p></section></div>}
    {replay && <Replay hand={replay.hand} events={replay.events} onClose={()=>setReplay(undefined)}/>}
  </div>;
}

createRoot(document.getElementById('root')!).render(<React.StrictMode><App/></React.StrictMode>);
