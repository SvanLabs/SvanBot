import type React from 'react';
import type { ReactNode } from 'react';
import { openPlayerCard } from './playercard';

/** Every player or bot name on the dashboard goes through here (0297): an opponent opens its scout view,
 * one of our bots switches the dashboard to that bot. The shell registers who is ours and every name it
 * knows (`registerNames`) on each render, so a name is recognised wherever it appears. */
let ours = new Set<string>();
let known: string[] = [];
let knownPattern: RegExp | null = null;

/** The event the shell answers by selecting one of our bots. */
export const SELECT_BOT_EVENT = 'sv-select-bot';

/** Called by the shell: our fleet's names and every other name it knows (opponents with profiles). */
export function registerNames(ourNames: string[], others: string[]) {
  const all = [...new Set([...ourNames, ...others].filter(n => n.length >= 2))].sort((a, b) => b.length - a.length);
  if (all.join('\u0000') === known.join('\u0000') && ourNames.every(n => ours.has(n)) && ours.size === ourNames.length) return;
  ours = new Set(ourNames);
  known = all;
  const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  // Longest first, and only as a whole word, so "Svanar" never matches inside "SvanarX".
  knownPattern = all.length ? new RegExp(`(?<![\\w])(${all.map(escape).join('|')})(?![\\w])`, 'g') : null;
}

export function isOurs(name: string): boolean {
  return ours.has(name);
}

function open(name: string) {
  if (ours.has(name)) window.dispatchEvent(new CustomEvent(SELECT_BOT_EVENT, { detail: name }));
  else openPlayerCard(name);
}

/** A clickable name. Inside a clickable row it acts alone: the row's own click and Enter do not fire.
 * `nested` renders a focusable span with button semantics for names inside another button, where a
 * second `<button>` would be invalid markup. */
export function PlayerName({ name, children, className = '', nested = false }: { name?: string | null; children?: ReactNode; className?: string; nested?: boolean }) {
  if (!name) return <>{children ?? '—'}</>;
  const own = ours.has(name);
  const props = {
    className: `player-name ${own ? 'ours' : ''} ${className}`.trim(),
    title: own ? `Show ${name} on the dashboard` : `Open ${name}'s scout view`,
    onClick: (e: React.MouseEvent) => { e.stopPropagation(); e.preventDefault(); open(name); },
  };
  if (nested) {
    return <span role="button" tabIndex={0} {...props} onKeyDown={e => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); open(name); }
    }}>{children ?? name}</span>;
  }
  // Only the keys that activate the button stop here, so a row's own Enter does not fire too; every
  // other key (Escape closing a dialog, arrow keys) must still reach the page's listeners.
  return <button type="button" {...props} onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') e.stopPropagation(); }}>{children ?? name}</button>;
}

/** Free text (a monitor line, a story) with every known name in it made clickable. */
export function NamesIn({ text }: { text: string }) {
  if (!knownPattern || !text) return <>{text}</>;
  const parts = text.split(knownPattern);
  // `split` with one capture group alternates text, name, text, name, ...
  return <>{parts.map((p, i) => i % 2 === 1 ? <PlayerName key={i} name={p}/> : p)}</>;
}
