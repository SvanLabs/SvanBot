import type { MonitorState } from './types';
import { NamesIn } from './playername';
import { usePoll } from './api';

const KIND_CLASS: Record<string, string> = { BIGWIN: 'positive', BIGLOSS: 'negative', NEMESIS: 'negative', STALL: 'amber', ERROR: 'negative', MONITOR: 'amber' };
const pct = (v: number | null) => v == null ? '—' : `${v.toFixed(1)}%`;
/** The monitor writes `HH:MM` from its own clock (the `monitor` binary), with no zone or date, so the
 *  browser cannot re-zone it (0295): the panel says whose clock it is instead of implying the viewer's. */
const HOST_CLOCK = 'The results monitor writes this wall time from the machine that runs it, with no zone or date, so it cannot be shown in your own zone.';

/** Results monitor (the `monitor` binary) plus machine pressure, replay records and the season check. */
export function ResultsMonitor() {
  // `usePoll` asks again the moment a session exists; a bare interval left this panel on "unavailable"
  // for up to half a minute after the login.
  const { data, error } = usePoll<MonitorState>('/monitor', 30_000);
  const failed = error != null;
  if (!data) return <p className="footnote">{failed ? 'Monitor data unavailable.' : 'Loading monitor…'}</p>;
  const m = data.monitor;
  const parts = (text?: string) => (text || '').split(' | ');
  return <div className="results-monitor">
    <div className="health-list">
      <div><span>Monitor</span><b className={m.running ? 'positive' : 'negative'}>{m.running ? 'Running' : 'Not writing — check scripts/start.sh'}</b></div>
      <div><span>Pressure · 60 s</span><b className="nowrap">CPU {pct(data.pressure.cpu)} · I/O {pct(data.pressure.io)} · mem {pct(data.pressure.memory)}</b></div>
      <div><span>Big decisions recorded</span>{data.replays.recorded == null
        ? <b className="negative" title={data.replays.error ?? undefined}>unreadable — the replay store read failed</b>
        : <b>{data.replays.recorded.toLocaleString('en-US')} (kept {data.replays.keep_days} days)</b>}</div>
      {data.season_check && <div><span>Season check</span><b className={data.season_check.failures.length ? 'negative' : 'positive'}>{data.season_check.result}</b></div>}
    </div>
    {m.summary && <section aria-label="Latest summary"><h3>Last 30 minutes · <span className="subtle" title={HOST_CLOCK}>{m.summary.time} host time</span></h3><ul className="monitor-parts">{parts(m.summary.text).map((p, i) => <li key={i}><NamesIn text={p}/></li>)}</ul></section>}
    {m.opponents && <section aria-label="Opponent intelligence"><h3>Opponents · <span className="subtle" title={HOST_CLOCK}>{m.opponents.time} host time</span></h3><ul className="monitor-parts">{parts(m.opponents.text).map((p, i) => <li key={i}><NamesIn text={p}/></li>)}</ul></section>}
    <section aria-label="Monitor alerts"><h3>Alerts</h3>
      {m.alerts.length ? <ul className="monitor-alerts">{m.alerts.slice(0, 12).map((a, i) => <li key={i}><span className="mono" title={HOST_CLOCK}>{a.time}</span> <b className={KIND_CLASS[a.kind] || ''}>{a.kind}</b> <NamesIn text={a.text}/></li>)}</ul>
        : <p className="footnote">No big wins or losses, nemeses, stalls or errors in the recent log.</p>}
    </section>
  </div>;
}
