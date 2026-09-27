/** Host check (0241): the facts about this machine that matter for the fleet (microcode, huge pages,
 * memory, free space on the SSD and the archive disk, SSD TRIM), read-only, each with the operator's
 * command when it is off. Nothing here changes the host. */
import { CircleCheck, CircleHelp, TriangleAlert } from 'lucide-react';
import type { HostCheck, HostState } from './types';
import { usePoll } from './api';

function StatusIcon({ status }: { status: HostCheck['status'] }) {
  if (status === 'ok') return <CircleCheck size={13} className="positive" aria-label="ok"/>;
  if (status === 'warn') return <TriangleAlert size={13} className="amber" aria-label="needs attention"/>;
  return <CircleHelp size={13} className="muted" aria-label="for context"/>;
}

export function HostPanel() {
  const { data, error } = usePoll<HostState>('/host', 60_000);
  if (!data) return <div className="host-panel"><p className="footnote">{error ? `Host check unavailable: ${error}` : 'Reading the host…'}</p></div>;
  const warnings = data.checks.filter(c => c.status === 'warn').length;
  return <div className="host-panel">
    <p className={`host-summary ${warnings ? 'amber' : 'positive'}`}>{warnings ? `${warnings} thing${warnings > 1 ? 's' : ''} to look at` : 'Everything as recommended'}</p>
    <ul className="host-list">{data.checks.map(c => <li key={c.key} className={`host-${c.status}`}>
      <div><StatusIcon status={c.status}/><span>{c.label}</span><b>{c.value}</b></div>
      {c.advice && <code>{c.advice}</code>}
    </li>)}</ul>
    {error && <p className="footnote negative">Last refresh failed: {error}</p>}
  </div>;
}
