import { useEffect, useState } from 'react';
import { AlertTriangle, ArrowDownRight, ArrowUpRight, CalendarDays, ChevronDown, History, RotateCcw } from 'lucide-react';
import { StaleNote, usePoll } from './api';
import { format, signed, Panel } from './ui';

type Kind = 'season' | 'release' | 'operation' | 'promotion' | 'warning' | 'error';
interface Hour { ts: number; hands: number; priced: number; net: number; ev_net: number; ev_net_sq: number }
interface Mark { id: string; kind: Kind; ts: number; title: string; detail: string; source: Record<string, unknown> }
interface TimelineData {
  scope: { scoped: boolean; number?: number | null };
  from: number;
  until: number;
  window_limited: boolean;
  release_log_available: boolean;
  operations_ledger_available: boolean;
  hours: Hour[];
  marks: Mark[];
  updated: number;
}
interface DetailHand { slot: number | null; bot: string; stored_bot: string; hand_id: string; ts: number; net: number | null; ev_net: number | null }
interface HourDetail { hour: number; hands: DetailHand[] }

const HOUR = 3600;
const utc = (ts: number, options: Intl.DateTimeFormatOptions) => new Date(ts * 1000).toLocaleString('en-GB', { timeZone: 'UTC', ...options });
const date = (ts: number) => new Date(ts * 1000).toISOString().slice(0, 10);
const hourName = (ts: number) => `${utc(ts, { day: '2-digit', month: 'short', hour: '2-digit', minute: '2-digit', hour12: false })} UTC`;
const sourceName: Record<Kind, string> = { season: 'Season record', release: 'Release log', operation: 'Operator event', promotion: 'Learner experiment', warning: 'Stored event', error: 'Stored event' };

function total(hours: Hour[]) {
  return hours.reduce((a, h) => ({ hands: a.hands + h.hands, priced: a.priced + h.priced, net: a.net + h.net, ev: a.ev + h.ev_net, sq: a.sq + h.ev_net_sq }), { hands: 0, priced: 0, net: 0, ev: 0, sq: 0 });
}

function band(sum: ReturnType<typeof total>) {
  if (sum.priced < 2) return null;
  const spread = Math.max(0, (sum.sq - sum.ev * sum.ev / sum.priced) / (sum.priced - 1));
  return 1.96 * Math.sqrt(spread / sum.priced);
}

function MarkDetail({ mark }: { mark: Mark }) {
  const source = mark.source;
  const record = source.record as Record<string, unknown> | undefined;
  return <div className={`axis-mark-detail ${mark.kind}`} role="region" aria-label={`${mark.title} source record`}>
    <div><strong>{mark.title}</strong><time>{hourName(mark.ts)}</time><span>{sourceName[mark.kind]}</span></div>
    <p>{mark.detail}</p>
    {mark.kind === 'release' && <small>Commit {String(source.commit || 'unknown')} · release log line {String(source.line || '?')}</small>}
    {mark.kind === 'operation' && <small>Operator event ledger line {String(source.ledger_line || '?')} · ticket {String(record?.ticket || '?')} · {String(record?.section || 'source record')}</small>}
    {mark.kind === 'promotion' && <small>Experiment {String(source.id || 'unknown')} · {format(Number(source.hands) || 0)} paired simulated hands · status {String(source.status || 'unknown')}</small>}
    {(mark.kind === 'warning' || mark.kind === 'error') && <small>Event #{String(source.id || '?')} · {String(source.bot || 'fleet')} · {String(source.level || mark.kind)}</small>}
    {mark.kind === 'season' && <small>Season ID {String(source.season_id || 'unknown')} · started {hourName(mark.ts)}</small>}
  </div>;
}

/** Results view: 24 visible hourly bins on a season-wide scrubber. The server reads only numeric
 * hand columns for the axis; an exact one-hour hand list is fetched after selection. */
export function TimelinePanel({ onReplay }: { onReplay: (slot: number, handId: string) => void }) {
  const poll = usePoll<TimelineData>('/timeline', 120000);
  const data = poll.data;
  const [chosen, setChosen] = useState<number | null>(null);
  const [markId, setMarkId] = useState<string | null>(null);
  const [shown, setShown] = useState(24);
  useEffect(() => {
    if (!data?.hours.length) return;
    if (chosen == null || chosen < data.from || chosen >= data.until) setChosen(data.hours[data.hours.length - 1].ts);
  }, [data?.from, data?.until, data?.hours.length, chosen]);
  const selected = chosen ?? data?.hours[data.hours.length - 1]?.ts ?? null;
  const detail = usePoll<HourDetail>(selected == null ? null : `/timeline/hour/${selected}`, 120000);
  const rows = detail.data?.hour === selected ? detail.data.hands : undefined;
  const select = (ts: number, mark: string | null = null) => { setChosen(ts); setMarkId(mark); setShown(24); };

  if (!data) return <Panel title="Unified event timeline" icon={<History size={15}/>} className="axis-panel"><div className="axis-empty">{poll.error ? `Timeline unavailable: ${poll.error}` : 'Loading the season timeline…'}</div></Panel>;
  if (!data.hours.length || selected == null) return <Panel title="Unified event timeline" icon={<History size={15}/>} className="axis-panel"><div className="axis-empty">No timeline hours are available.</div></Panel>;

  const index = Math.max(0, data.hours.findIndex(h => h.ts === selected));
  const dayStart = Math.floor(selected / 86400) * 86400;
  const visible = data.hours.filter(h => h.ts >= dayStart && h.ts < dayStart + 86400);
  const recent = data.hours.slice(Math.max(0, index - 23), index + 1);
  const prior = data.hours.slice(Math.max(0, index - 47), Math.max(0, index - 23));
  const now = total(recent);
  const before = total(prior);
  const maxNet = Math.max(1, ...visible.map(h => Math.abs(h.net)));
  const marksByHour = new Map<number, Mark[]>();
  for (const m of data.marks) {
    const key = Math.floor(m.ts / HOUR) * HOUR;
    marksByHour.set(key, [...(marksByHour.get(key) || []), m]);
  }
  const hour = data.hours[index];
  const selectedMarks = marksByHour.get(selected) || [];
  const selectedMark = selectedMarks.find(m => m.id === markId) || null;
  const rate = now.priced ? now.ev / now.priced : null;
  const priorRate = before.priced ? before.ev / before.priced : null;
  const uncertainty = band(now);

  return <Panel title="Unified event timeline" icon={<History size={15}/>} className="axis-panel" aside={<span className="tag">{data.scope.scoped ? `SEASON ${data.scope.number ?? ''}` : 'LATEST 14 DAYS'} · UTC</span>}>
    <div className="axis-body">
      <StaleNote poll={poll}/>
      {data.window_limited && <p className="footnote amber">This season is longer than the 21-day timeline window; earlier hours are omitted.</p>}
      {!data.release_log_available && <p className="footnote amber">Release log unavailable; release marks are missing.</p>}
      {!data.operations_ledger_available && <p className="footnote amber">Operator event ledger unavailable; operational change marks are missing.</p>}
      <div className="axis-summary" aria-label="24-hour result around selected time">
        <div><span>24 HOURS ENDING {hourName(selected).toUpperCase()}</span><b className={now.net < 0 ? 'negative' : 'positive'}>{signed(now.net)} chips</b><small>{format(now.hands)} hands · {format(now.priced)} with recorded results</small></div>
        <div><span>ALL-IN EV ADJUSTED</span><b className={rate != null && rate < 0 ? 'negative' : 'positive'}>{rate == null ? 'Unavailable' : `${signed(rate, 1)} chips/hand`}</b><small>{uncertainty == null ? '' : `Approx. hand-level 95% band ±${format(uncertainty, 1)} · `}{priorRate == null ? 'No prior 24-hour comparison' : `prior 24 h ${signed(priorRate, 1)} chips/hand`}</small></div>
        <div><span>SELECTED HOUR</span><b className={hour.net < 0 ? 'negative' : 'positive'}>{signed(hour.net)} chips</b><small>{hour.hands ? `${format(hour.hands)} hands · ${hour.priced} priced` : 'No recorded hands · cause not inferred'}</small></div>
      </div>
      <p className="axis-caution">Hourly and 24-hour results describe stored hands. Short swings do not establish a strategy change; open the hands and marks to investigate.</p>
      <div className="axis-controls">
        <label><CalendarDays size={14}/> Jump to UTC date <input type="date" aria-label="Jump to UTC date" min={date(data.from)} max={date(data.until - HOUR)} value={date(selected)} onChange={event => {
          const lastHour = Date.parse(`${event.target.value}T23:00:00Z`) / 1000;
          if (Number.isFinite(lastHour)) select(data.hours.reduce((best, h) => Math.abs(h.ts - lastHour) < Math.abs(best - lastHour) ? h.ts : best, data.hours[0].ts));
        }}/></label>
        <button className="text-button" onClick={() => select(data.hours[data.hours.length - 1].ts)}><RotateCcw size={13}/>Latest hour</button>
        <span>{hourName(visible[0].ts)} – {hourName(visible[visible.length - 1].ts)}</span>
      </div>
      <input className="axis-scrubber" aria-label="Scrub season by hour" type="range" min={0} max={data.hours.length - 1} value={index} onChange={event => select(data.hours[Number(event.target.value)].ts)}/>
      <div className="axis-chart" role="group" aria-label="Hourly fleet net chips and event marks">
        {visible.map(h => {
          const marks = marksByHour.get(h.ts) || [];
          const magnitude = Math.max(2, Math.round(Math.abs(h.net) / maxNet * 42));
          return <button key={h.ts} type="button" className={`axis-hour ${h.ts === selected ? 'selected' : ''} ${h.hands ? '' : 'silent'}`} aria-pressed={h.ts === selected}
            aria-label={`${hourName(h.ts)}: ${h.hands} hands, ${signed(h.net)} chips, ${marks.length} marks`}
            onClick={() => select(h.ts)} title={`${hourName(h.ts)} · ${h.hands} hands · ${signed(h.net)} chips · ${marks.length} marks`}>
            <span className="axis-mark-count">{marks.length ? marks.length : ''}</span>
            <span className="axis-plot"><i className={h.net < 0 ? 'down' : 'up'} style={{ height: `${magnitude}%` }}/></span>
            <span className="axis-tick">{utc(h.ts, { hour: '2-digit', hour12: false })}</span>
          </button>;
        })}
      </div>
      <div className="axis-legend"><span><i className="up"/>Net gain</span><span><i className="down"/>Net loss</span><span><i className="silent"/>No hands</span><span><i className="mark"/>Release, operator change, promotion, season or issue</span></div>
      <div className="axis-drill">
        <section className="axis-events"><h3>At {hourName(selected)} <small>{selectedMarks.length} marks</small></h3>
          {selectedMarks.length ? <div className="axis-mark-list">{selectedMarks.map(m => <button key={m.id} className={`${m.kind} ${m.id === selectedMark?.id ? 'active' : ''}`} onClick={() => setMarkId(m.id)}><span>{m.kind === 'error' || m.kind === 'warning' ? <AlertTriangle size={13}/> : m.kind === 'release' ? <ArrowUpRight size={13}/> : <ArrowDownRight size={13}/>}</span><b>{m.title}</b><small>{utc(m.ts, { hour: '2-digit', minute: '2-digit', hour12: false })} · {sourceName[m.kind]}</small></button>)}</div> : <p className="subtle">No release, promotion, season, warning or error record in this hour.</p>}
          {selectedMark && <MarkDetail mark={selectedMark}/>}
        </section>
        <section className="axis-hands"><h3>Hands behind this hour <small>{hour.hands} stored</small></h3>
          <StaleNote poll={detail}/>
          {!rows ? <p className="subtle">{detail.error ? `Hand rows unavailable: ${detail.error}` : 'Loading exact hand rows…'}</p> : rows.length ? <><div className="axis-hand-list">{rows.slice(0, shown).map(h => <button key={`${h.stored_bot}:${h.hand_id}`} onClick={() => { if (h.slot != null) onReplay(h.slot, h.hand_id); }} disabled={h.slot == null}><span>{utc(h.ts, { hour: '2-digit', minute: '2-digit', hour12: false })} · {h.bot}</span><b className={h.net != null && h.net < 0 ? 'negative' : 'positive'}>{h.net == null ? 'net pending' : signed(h.net)}</b><small>#{h.hand_id.slice(0, 10)}</small></button>)}</div>{shown < rows.length && <button className="axis-more" onClick={() => setShown(shown + 24)}><ChevronDown size={13}/>Show more hands ({rows.length - shown} remaining)</button>}</> : <p className="subtle">No stored hands in this hour. The timeline does not infer why play stopped.</p>}
        </section>
      </div>
    </div>
  </Panel>;
}
