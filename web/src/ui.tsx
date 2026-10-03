/** Shared dashboard primitives: formatters, the seat read, table themes, cards, panels and empty states. */
import React, { useState } from 'react';
import { Activity, ChevronDown, ChevronRight, Spade } from 'lucide-react';
import { panelHelp } from './help';
import type { SeatRead } from './types';
import { request } from './api';
import { fmt, pct, sgn, SUITS } from './format';
import { readLocal, writeLocal } from './storage';

export const format = (value: number | null | undefined, digits = 0) => fmt(value, digits);
export const signed = (value: number | null | undefined, digits = 0) => sgn(value, digits);
export const percent = (value: number | null | undefined) => pct(value, 1);
export const ago = (stamp: number) => { const m = Math.max(0, Math.round((Date.now() / 1000 - stamp) / 60)); return m < 1 ? 'just now' : m < 60 ? `${m} min ago` : m < 48 * 60 ? `${Math.floor(m / 60)} h ${m % 60} min ago` : `${Math.floor(m / 1440)} days ago`; };
export const suitMap = SUITS;
/** One-line read of an opponent at the table, and the fuller tooltip behind it (0212). */
export const pct0 = (v: number) => `${Math.round(v * 100)}`;
export const readLine = (r: SeatRead) => `${r.style} · ${pct0(r.vpip)}/${pct0(r.pfr)}`;
export const readTitle = (r: SeatRead) => {
  const lines = [`${format(r.hands)} hands observed · ${r.style}`, `VPIP ${pct0(r.vpip)}% · PFR ${pct0(r.pfr)}% · 3-bet ${pct0(r.three_bet)}%`, `Folds to a bet: flop ${pct0(r.fold_vs_bet[0])}% · turn ${pct0(r.fold_vs_bet[1])}% · river ${pct0(r.fold_vs_bet[2])}%`];
  if (r.fold_offset) lines.push(`Heads-up, folds to our bets ${r.fold_offset < 0 ? 'less' : 'more'} than the model prices (logit ${r.fold_offset > 0 ? '+' : ''}${r.fold_offset.toFixed(2)}), and is priced that way`);
  if (r.size_tell) lines.push(`River sizing tell ${r.size_tell > 0 ? '+' : ''}${r.size_tell.toFixed(2)}: ${r.size_tell > 0 ? 'bets bigger with stronger hands' : 'bets bigger with weaker hands'} than the pool, and is read that way`);
  if (r.response_ratio) lines.push(`Against our bets, versus the neural model: folds ×${r.response_ratio[0].toFixed(2)} · calls ×${r.response_ratio[1].toFixed(2)} · raises ×${r.response_ratio[2].toFixed(2)}`);
  return lines.join('\n');
};
export const dotClass = (mode?: string) => mode === 'playing' ? '' : mode === 'connecting' ? 'connecting' : mode === 'paused' ? 'paused' : 'offline';

export type TableTheme = 'felt' | 'arena' | 'midnight';
export const TABLE_THEMES: TableTheme[] = ['felt', 'arena', 'midnight'];

export function ThemeToggle({theme, onChange}: {theme: TableTheme; onChange: (theme: TableTheme) => void}) {
  return <div className="theme-toggle" role="group" aria-label="Table theme">
    {TABLE_THEMES.map(t => <button key={t} className={theme === t ? 'active' : ''} aria-pressed={theme === t} onClick={() => onChange(t)}>{t[0].toUpperCase()}{t.slice(1)}</button>)}
  </div>;
}

/** Which complete look the control room wears (#728). `dark` is the base theme style.css was written
 *  against; `light` is the same layout re-inked through the theme tokens. The switch is the
 *  `data-theme` attribute on the document root, so no component forks on it. */
export type SiteTheme = 'dark' | 'light';
export const SITE_THEMES: SiteTheme[] = ['dark', 'light'];

export function SiteThemePicker({theme, onChange}: {theme: SiteTheme; onChange: (theme: SiteTheme) => void}) {
  return <div className="theme-toggle" role="group" aria-label="Control room theme">
    {SITE_THEMES.map(t => <button key={t} className={theme === t ? 'active' : ''} aria-pressed={theme === t} onClick={() => onChange(t)}>{t[0].toUpperCase()}{t.slice(1)}</button>)}
  </div>;
}

export const api = request;

export function Card({card, small = false}: {card?: string; small?: boolean}) {
  return <span className={`playing-card ${small ? 'small' : ''} ${!card ? 'back' : ''} ${card && 'hd'.includes(card[1]) ? 'red' : ''}`} aria-label={card || 'Hidden card'}>{card ? <><b>{card[0]}</b><span>{suitMap[card[1]]}</span></> : <Spade size={small ? 12 : 19} />}</span>;
}

/** Which widget a Panel sits in (the board's widget id). A Panel outside a board has none and is
 *  keyed by its title, the way every panel was before #729. */
export const WidgetIdContext = React.createContext<string | undefined>(undefined);

/** Panel collapse is a browser-local reading preference (#729), kept in one small store so the
 *  workspace's collapse-all can reach every panel on screen. The key is the widget id where there is
 *  one: a retitled panel keeps its state and two same-titled panels no longer share it. The old
 *  title-based key is read as a fallback, so a state saved before #729 still applies. */
const collapseState = new Map<string, boolean>();
const collapseListeners = new Set<() => void>();
const subscribeCollapse = (listener: () => void) => { collapseListeners.add(listener); return () => { collapseListeners.delete(listener); }; };
const collapseKey = (id: string) => `svan-panel-collapsed:${id}`;
function isCollapsed(key: string, legacyTitle?: string): boolean {
  const known = collapseState.get(key);
  if (known !== undefined) return known;
  const stored = readLocal(collapseKey(key)) ?? (legacyTitle ? readLocal(collapseKey(legacyTitle)) : null);
  const collapsed = stored === 'true';
  collapseState.set(key, collapsed);
  return collapsed;
}
export function setPanelCollapsed(key: string, collapsed: boolean) {
  collapseState.set(key, collapsed);
  writeLocal(collapseKey(key), String(collapsed));
  collapseListeners.forEach(listener => listener());
}
/** Collapse or expand every panel named by `ids` — the widgets of one view. */
export function setPanelsCollapsed(ids: string[], collapsed: boolean) { for (const id of ids) setPanelCollapsed(id, collapsed); }
/** Whether every named panel is collapsed; the caller re-renders when any of them is toggled. */
export function useAllCollapsed(ids: string[]): boolean {
  return React.useSyncExternalStore(subscribeCollapse, () => ids.length > 0 && ids.every(id => isCollapsed(id)));
}

export function Panel({title, icon, aside, children, className = ''}: {title:string;icon:React.ReactNode;aside?:React.ReactNode;children:React.ReactNode;className?:string}) {
  const [showHelp, setShowHelp] = useState(false);
  const helpId = React.useId();
  const widgetId = React.useContext(WidgetIdContext);
  const key = widgetId ?? title;
  const collapsed = React.useSyncExternalStore(subscribeCollapse, () => isCollapsed(key, widgetId ? title : undefined));
  const toggle = () => setPanelCollapsed(key, !collapsed);
  // The whole bar is the toggle (#729); the chevron stays the explicit affordance, and a control in
  // the heading — the info button, a link in `aside` — must not collapse the panel under it.
  return <section className={`panel ${className} ${collapsed ? 'panel-collapsed' : ''}`}><div className="panel-heading" onClick={event => { if (!(event.target as HTMLElement).closest('button, a')) toggle(); }}><h2>{icon}{title}</h2><div className="panel-heading-actions">{aside}<button className="icon-button panel-info" aria-label={`About ${title}`} aria-expanded={showHelp} aria-controls={helpId} onClick={event => {event.stopPropagation();setShowHelp(!showHelp);}}><span aria-hidden="true">i</span></button><button className="icon-button" aria-label={`${collapsed ? 'Expand' : 'Minimize'} ${title}`} aria-expanded={!collapsed} onClick={event => {event.stopPropagation();toggle();}}>{collapsed ? <ChevronRight size={15}/> : <ChevronDown size={15}/>}</button></div></div>{showHelp && <div className="panel-explanation" id={helpId} role="region" aria-label={`${title} explanation`}><p>{panelHelp[title]}</p><a href="#help">How the bot works →</a> <a href="#docs">Read the full docs →</a></div>}<div className="panel-content" hidden={collapsed}>{children}</div></section>;
}

export function Empty({title, detail, icon = <Activity size={23}/>}:{title:string;detail:string;icon?:React.ReactNode}) {
  return <div className="empty">{icon}<strong>{title}</strong><p>{detail}</p></div>;
}
