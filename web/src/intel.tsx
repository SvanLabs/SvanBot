/** Per-opponent reads (0222): every per-opponent correction live play applies, the held-out
 * evidence that installed it or kept it out, and the opponents it moves most. */
import { StaleNote, usePoll } from './api';
import { fmt, sgn } from './format';
import type { IntelFit, IntelOpponent, IntelState } from './types';
import { PlayerName } from './playername';

function verdict(f: IntelFit): { label: string; tone: string } {
  if (!f.stored) return { label: 'NOT FITTED YET', tone: 'muted' };
  if (f.active) return { label: `LIVE · ${f.installed} OPPONENTS`, tone: 'positive' };
  return { label: 'GATED · NOT INSTALLED', tone: 'amber' };
}

function FitRow({ fit }: { fit: IntelFit }) {
  const v = verdict(fit);
  const e = fit.evidence;
  return <li className="intel-fit">
    <div className="intel-fit-head"><b>{fit.title}</b><span className={`intel-verdict ${v.tone}`}>{v.label}</span></div>
    <p>{fit.reads}.</p>
    {e && <small>Held-out gain {e.gain_mnats >= 0 ? '+' : ''}{e.gain_mnats.toFixed(2)} ± {e.half_width_mnats.toFixed(2)} mnats per sample{e.n != null ? ` on ${fmt(e.n)}` : ''}{fit.active ? '' : ': installed only once the gain clears its 95% band'}</small>}
  </li>;
}


function OpponentRow({ o }: { o: IntelOpponent }) {
  const parts: string[] = [];
  if (o.fold_offset != null) parts.push(`folds ${o.fold_offset > 0 ? 'more' : 'less'} (${sgn(o.fold_offset, 2, true)})`);
  if (o.response_ratio) parts.push(`f/c/r ×${o.response_ratio.map(r => r.toFixed(2)).join('/')}`);
  if (o.size_tell != null) parts.push(`sizes ${o.size_tell > 0 ? 'up with strength' : 'up with weakness'} (${sgn(o.size_tell, 2, true)})`);
  return <li>
    <PlayerName className="opponent-link intel-name" name={o.name}/>
    <small>{fmt(o.hands)} hands observed · {parts.join(' · ') || 'no correction'}</small>
  </li>;
}

export function IntelPanel() {
  const poll = usePoll<IntelState>('/intel', 60000);
  const data = poll.data;
  if (!data) return <p className="subtle intel-empty">{poll.error ? `Unavailable: ${poll.error}` : 'Loading per-opponent reads…'}</p>;
  return <div className="intel">
    <StaleNote poll={poll} />
    <p className="footnote">Each opponent is read three ways on top of the shared model. A read goes live only while it predicts newer hands better than the shared model does, and the learner refits all three every cycle. Hands observed is the same evidence-weighted count the opponent list shows; click a name for the scout view.</p>
    <ul className="intel-fits">{data.fits.map(f => <FitRow key={f.id} fit={f} />)}</ul>
    <h3 className="intel-sub">Most-observed corrected opponents <small>{fmt(data.corrected_opponents)} in all</small></h3>
    {data.opponents.length
      ? <ul className="intel-opponents">{data.opponents.map(o => <OpponentRow key={o.name} o={o} />)}</ul>
      : <p className="subtle">No opponent is corrected yet: every read waits on its held-out gate.</p>}
  </div>;
}
