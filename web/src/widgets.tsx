import React, { useEffect, useRef, useState } from 'react';
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, EyeOff, Grip, Plus, RotateCcw } from 'lucide-react';
import { readLocal, writeLocal } from './storage';
import { request } from './api';
import { WidgetIdContext } from './ui';
import type { DashboardLayout } from './types';

export type Column = 'left' | 'center' | 'right';
/** The board's arrangement, typed by the API contract (`web/src/types.ts`): the same document the
 * server stores, so the board and the endpoint cannot drift. */
export type Layout = DashboardLayout;
export interface Widget { id: string; title: string; node: React.ReactNode }

const COLUMNS: Column[] = ['left', 'center', 'right'];
const STORAGE_KEY = 'svan-layout:v1';

/** Stored layout reconciled with the widgets that exist now: unknown ids are dropped, new widgets
 * land in their default column, so a release that adds or removes a panel never loses the layout. */
export function loadLayout(defaults: Layout): Layout {
  // The read itself cannot throw (storage.ts); the parse still can on a value that is not JSON.
  let stored: Partial<Layout> | undefined;
  try { stored = JSON.parse(readLocal(STORAGE_KEY) || 'null') || undefined; } catch { stored = undefined; }
  return reconcile(stored, defaults);
}

/** A layout from anywhere — this browser or the server (#729) — reconciled the same way. */
function reconcile(stored: Partial<Layout> | null | undefined, defaults: Layout): Layout {
  if (!stored) return clone(defaults);
  const known = new Set([...COLUMNS.flatMap(c => defaults[c]), ...defaults.hidden]);
  const seen = new Set<string>();
  const keep = (ids: unknown) => (Array.isArray(ids) ? ids : []).filter((id): id is string => typeof id === 'string' && known.has(id) && !seen.has(id) && (seen.add(id), true));
  const out: Layout = { left: keep(stored.left), center: keep(stored.center), right: keep(stored.right), hidden: keep(stored.hidden) };
  for (const column of COLUMNS) for (const id of defaults[column]) if (!seen.has(id)) { out[column].push(id); seen.add(id); }
  return out;
}

function clone(layout: Layout): Layout {
  return { left: [...layout.left], center: [...layout.center], right: [...layout.right], hidden: [...layout.hidden] };
}

function sameLayout(left: Layout, right: Layout) {
  return [...COLUMNS, 'hidden' as const].every(column => left[column].length === right[column].length && left[column].every((id, index) => id === right[column][index]));
}

function save(layout: Layout) {
  writeLocal(STORAGE_KEY, JSON.stringify(layout));
}

function locate(layout: Layout, id: string): [Column, number] | undefined {
  for (const column of COLUMNS) { const index = layout[column].indexOf(id); if (index >= 0) return [column, index]; }
  return undefined;
}

/** Move `id` to `column` before `beforeId` (end of column when absent). */
export function moveWidget(layout: Layout, id: string, column: Column, beforeId?: string): Layout {
  if (id === beforeId) return layout;
  const next = clone(layout);
  for (const c of COLUMNS) next[c] = next[c].filter(x => x !== id);
  next.hidden = next.hidden.filter(x => x !== id);
  const at = beforeId ? next[column].indexOf(beforeId) : -1;
  next[column].splice(at < 0 ? next[column].length : at, 0, id);
  return next;
}

function step(layout: Layout, id: string, direction: 'up' | 'down' | 'left' | 'right'): Layout {
  const found = locate(layout, id);
  if (!found) return layout;
  const [column, index] = found;
  if (direction === 'up' || direction === 'down') {
    const target = index + (direction === 'up' ? -1 : 1);
    if (target < 0 || target >= layout[column].length) return layout;
    const next = clone(layout);
    [next[column][index], next[column][target]] = [next[column][target], next[column][index]];
    return next;
  }
  const columnIndex = COLUMNS.indexOf(column) + (direction === 'left' ? -1 : 1);
  if (columnIndex < 0 || columnIndex >= COLUMNS.length) return layout;
  const targetColumn = COLUMNS[columnIndex];
  return moveWidget(layout, id, targetColumn, layout[targetColumn][Math.min(index, layout[targetColumn].length)]);
}

const columnClass: Record<Column, string> = { left: 'left-column', center: 'center-column', right: 'right-column' };
const columnLabel: Record<Column, string> = { left: 'left', center: 'centre', right: 'right' };

/** The dashboard's views (0237): each shows a subset of the widgets in the user's own arrangement,
 * so the page opens on what matters now instead of one 6,600 px scroll. `all` is the full board. */
export const VIEWS: { id: string; label: string; widgets: string[] | null }[] = [
  { id: 'live', label: 'Live', widgets: ['table', 'ticker', 'recent-hands', 'fleet-race', 'monitor', 'highlights', 'activity'] },
  { id: 'opponents', label: 'Opponents', widgets: ['opponents', 'intel', 'rivals', 'leaks', 'ranges', 'starting-hands'] },
  { id: 'learning', label: 'Learning', widgets: ['autonomy', 'experiments', 'experiment-mode', 'calibration', 'accuracy', 'wiring', 'champion'] },
  { id: 'results', label: 'Results', widgets: ['performance', 'season-race', 'badges', 'season', 'stories', 'fleet-race', 'highlights'] },
  { id: 'system', label: 'System', widgets: ['health', 'updates', 'host', 'monitor', 'activity', 'privacy'] },
  { id: 'all', label: 'All', widgets: null },
];

/** The remembered view, falling back to Live. */
export function loadView(): string {
  const v = readLocal('svan-view');
  return v && VIEWS.some(view => view.id === v) ? v : 'live';
}

/** Tab bar for the dashboard views; `extra` names widgets the user placed that a view does not list. */
export function ViewTabs({ view, onChange }: { view: string; onChange: (id: string) => void }) {
  const select = (id: string) => { onChange(id); writeLocal('svan-view', id); };
  return <nav className="view-tabs" role="tablist" aria-label="Dashboard views">
    {VIEWS.map(v => <button key={v.id} role="tab" aria-selected={view === v.id} className={view === v.id ? 'active' : ''} onClick={() => select(v.id)}
      onKeyDown={e => {
        const i = VIEWS.findIndex(x => x.id === view);
        if (e.key === 'ArrowRight') { e.preventDefault(); select(VIEWS[(i + 1) % VIEWS.length].id); }
        if (e.key === 'ArrowLeft') { e.preventDefault(); select(VIEWS[(i + VIEWS.length - 1) % VIEWS.length].id); }
      }}>{v.label}</button>)}
  </nav>;
}

export function WidgetBoard({ widgets, defaults, editing, onDoneEditing, view = 'all' }: { widgets: Widget[]; defaults: Layout; editing: boolean; onDoneEditing: () => void; view?: string }) {
  const [layout, setLayoutState] = useState(() => loadLayout(defaults));
  const [dragging, setDragging] = useState<string>();
  const [target, setTarget] = useState<{column: Column; beforeId?: string}>();
  const press = React.useRef<{id: string; x: number; y: number; pointer: number} | undefined>(undefined);
  const byId = new Map(widgets.map(w => [w.id, w]));
  // The arrangement lives on the server (#729) so it survives a browser change; this browser renders
  // its own copy at once, keeps it when the endpoint cannot be reached, and an answer that arrives
  // after the operator has already arranged something does not overwrite it.
  const arranged = useRef(false);
  useEffect(() => {
    let alive = true;
    request<DashboardLayout | null>('/layout').then(stored => {
      if (!alive || !stored || arranged.current) return;
      const next = reconcile(stored, defaults);
      setLayoutState(next);
      save(next); // remember it here too, so the next load has the arrangement even without the server
    }).catch(() => { /* no endpoint: the browser-local layout stands */ });
    return () => { alive = false; };
    // Mount only: the first `defaults` is the one this mount reconciles against (main.tsx passes a
    // fresh literal each render, and a later one is the same board).
  }, []);
  const setLayout = (next: Layout) => { arranged.current = true; setLayoutState(next); save(next); request('/layout', next).catch(() => {}); };

  // Pointer-driven dragging (mouse, pen and touch alike): the dragged widget ignores hit-testing,
  // so the element under the pointer is the drop target; the upper half of a widget inserts before
  // it, the lower half after it, empty column space appends.
  const targetAt = (x: number, y: number): {column: Column; beforeId?: string} | undefined => {
    const element = document.elementFromPoint(x, y) as HTMLElement | null;
    const columnElement = element?.closest<HTMLElement>('[data-column]');
    if (!columnElement) return undefined;
    const column = columnElement.dataset.column as Column;
    const widgetElement = element?.closest<HTMLElement>('[data-widget]');
    if (!widgetElement) return { column };
    const rect = widgetElement.getBoundingClientRect();
    const ids = layout[column].filter(id => id !== press.current?.id);
    const index = ids.indexOf(widgetElement.dataset.widget!);
    const beforeIndex = y < rect.top + rect.height / 2 ? index : index + 1;
    return { column, beforeId: ids[beforeIndex] };
  };
  const onPointerDown = (id: string) => (event: React.PointerEvent) => {
    if ((event.target as HTMLElement).closest('button') || event.button !== 0) return;
    press.current = { id, x: event.clientX, y: event.clientY, pointer: event.pointerId };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: React.PointerEvent) => {
    const p = press.current;
    if (!p || p.pointer !== event.pointerId) return;
    if (!dragging && Math.hypot(event.clientX - p.x, event.clientY - p.y) < 6) return;
    event.preventDefault();
    if (!dragging) setDragging(p.id);
    // Scroll while holding near the viewport's top or bottom edge, to reach widgets off-screen.
    const edge = 70;
    if (event.clientY < edge) window.scrollBy(0, -Math.ceil((edge - event.clientY) / 2));
    else if (event.clientY > window.innerHeight - edge) window.scrollBy(0, Math.ceil((event.clientY - window.innerHeight + edge) / 2));
    setTarget(targetAt(event.clientX, event.clientY));
  };
  const onPointerUp = (event: React.PointerEvent) => {
    const p = press.current;
    press.current = undefined;
    if (p && dragging) {
      const drop = targetAt(event.clientX, event.clientY) || target;
      if (drop) setLayout(moveWidget(layout, p.id, drop.column, drop.beforeId));
    }
    setDragging(undefined);
    setTarget(undefined);
  };
  // A view shows its widgets in the user's arrangement; arranging always shows the whole board.
  const listed = VIEWS.find(v => v.id === view)?.widgets;
  const shown = (id: string) => editing || !listed || listed.includes(id);
  const columns = COLUMNS.filter(column => editing || layout[column].some(id => byId.has(id) && shown(id)));
  const dropBefore = (id: string, column: Column) => dragging && target?.column === column && target.beforeId === id;
  const dropEnd = (column: Column) => dragging && target?.column === column && !target.beforeId;

  return <>
    {editing && <div className="layout-bar" role="toolbar" aria-label="Arrange widgets">
      <span>Drag widgets by their bar, or use the arrows. Your layout is saved on the server, with this browser's copy as the fallback.</span>
      {layout.hidden.length > 0 && <span className="layout-hidden">{layout.hidden.map(id => <button key={id} className="button" onClick={() => setLayout(moveWidget(layout, id, 'right'))}><Plus size={12}/>{byId.get(id)?.title || id}</button>)}</span>}
      <button className="button" onClick={() => setLayout(clone(defaults))}><RotateCcw size={12}/>Reset layout</button>
      <button className="button primary" onClick={onDoneEditing}>Done</button>
    </div>}
    <div className={`dashboard-grid ${sameLayout(layout, defaults) ? 'default-layout' : 'custom-layout'} ${editing ? 'arranging' : ''} view-${view} cols-${columns.length} ${COLUMNS.filter(c => !columns.includes(c)).map(c => `no-${c}`).join(' ')}`}>
      {columns.map(column => <div key={column} data-column={column} className={`${columnClass[column]} widget-column ${dropEnd(column) ? 'drop-end' : ''}`} aria-label={`${columnLabel[column]} column`}>
        {layout[column].map(id => {
          const widget = byId.get(id);
          if (!widget || !shown(id)) return null;
          return <div key={id} data-widget={id} className={`widget ${dragging === id ? 'dragging' : ''} ${dropBefore(id, column) ? 'drop-before' : ''}`}>
            {editing && <div className="widget-bar" onPointerDown={onPointerDown(id)} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}>
              <Grip size={13} aria-hidden="true"/><b>{widget.title}</b>
              <span className="widget-moves">
                <button className="icon-button" aria-label={`Move ${widget.title} up`} onClick={() => setLayout(step(layout, id, 'up'))}><ArrowUp size={13}/></button>
                <button className="icon-button" aria-label={`Move ${widget.title} down`} onClick={() => setLayout(step(layout, id, 'down'))}><ArrowDown size={13}/></button>
                <button className="icon-button" aria-label={`Move ${widget.title} to the left column`} disabled={column === 'left'} onClick={() => setLayout(step(layout, id, 'left'))}><ArrowLeft size={13}/></button>
                <button className="icon-button" aria-label={`Move ${widget.title} to the right column`} disabled={column === 'right'} onClick={() => setLayout(step(layout, id, 'right'))}><ArrowRight size={13}/></button>
                <button className="icon-button" aria-label={`Hide ${widget.title}`} onClick={() => { const next = moveWidget(layout, id, 'left'); next.left = next.left.filter(x => x !== id); next.hidden.push(id); setLayout(next); }}><EyeOff size={13}/></button>
              </span>
            </div>}
            <WidgetIdContext.Provider value={id}>{widget.node}</WidgetIdContext.Provider>
          </div>;
        })}
        {editing && <div className="widget-drop-zone">Drop here</div>}
      </div>)}
    </div>
  </>;
}
