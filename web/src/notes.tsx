/** The operator's notes scratchpad (#730): one plain-text panel in the board that saves itself while
 *  it is typed. The server holds the document (`GET`/`POST /api/notes`, `DashboardNotes` in
 *  `web/src/types.ts`); this browser's copy renders until the answer arrives and stands when the
 *  endpoint is down, so a dropped network or a reload never costs a keystroke — and the save state
 *  is always on screen, because a save that failed must not look like one that worked. */
import { useEffect, useRef, useState } from 'react';
import { NotebookPen } from 'lucide-react';
import { request, SESSION_EVENT } from './api';
import { readLocal, writeLocal } from './storage';
import { format, Panel } from './ui';
import type { DashboardNotes } from './types';

/** What this browser remembers, and whether its newest text reached the server. The flag is what
 *  survives a view switch, a reload or a dead endpoint: without it the next load would adopt an
 *  older stored note over keystrokes this browser never managed to send. */
const LOCAL_KEY = 'svan-notes:v1';
const LOCAL_DIRTY = 'svan-notes:unsaved';
/** Typing settles before a save; a blur or a page hide sends at once. */
const DEBOUNCE_MS = 1000;
/** A failed save retries from here, doubling to the ceiling. */
const RETRY_MIN_MS = 1000;
const RETRY_MAX_MS = 30_000;
/** The server refuses more than this (`crates/apps/bot/src/api/notes.rs` at the same number); the
 *  counter under the box says where the note stands. */
export const NOTES_MAX_CHARS = 20_000;

type SaveState = 'idle' | 'unsaved' | 'saving' | 'saved' | 'failed';
const STATUS: Record<SaveState, string> = {
  idle: '',
  unsaved: 'Unsaved changes',
  saving: 'Saving…',
  saved: 'Saved',
  failed: 'Not saved — retrying…',
};

export function NotesPanel() {
  const [text, setText] = useState(() => readLocal(LOCAL_KEY) ?? '');
  const [state, setState] = useState<SaveState>('idle');
  const latest = useRef(text);   // the newest text: what the debounce, the retries and the flush send
  // Text the server has not confirmed, including text an earlier session could not send: it must be
  // pushed, and a blur or a page hide flushes it, before the panel has been typed in at all.
  const dirty = useRef(readLocal(LOCAL_DIRTY) === '1');
  const touched = useRef(false); // the operator has typed here: a late server answer must not overwrite it
  const busy = useRef(false);    // one save in flight at a time, so an older one cannot land last
  const attempt = useRef(0);
  const timer = useRef<number | undefined>(undefined);

  const schedule = (wait: number) => { window.clearTimeout(timer.current); timer.current = window.setTimeout(() => void save(), wait); };
  const markDirty = (unsaved: boolean) => { dirty.current = unsaved; writeLocal(LOCAL_DIRTY, unsaved ? '1' : '0'); };

  /** Send the newest text now and say so on screen; a failure stays visible and schedules a retry. */
  async function save() {
    window.clearTimeout(timer.current);
    if (busy.current) { schedule(DEBOUNCE_MS); return; }   // an older save is in flight: this one waits its turn
    busy.current = true;
    const value = latest.current;
    setState('saving');
    try {
      await request<DashboardNotes>('/notes', { text: value });
      attempt.current = 0;
      // Keystrokes that arrived while this was in flight are not this answer's to report: they stay
      // unsaved, and the debounce their own edit scheduled sends them.
      if (latest.current === value) { markDirty(false); setState('saved'); }
    } catch {
      attempt.current += 1;
      setState('failed');
      schedule(Math.min(RETRY_MIN_MS * 2 ** Math.min(attempt.current - 1, 5), RETRY_MAX_MS));
    } finally { busy.current = false; }
  }

  const edit = (value: string) => {
    latest.current = value;
    touched.current = true;
    markDirty(true);
    attempt.current = 0;   // a fresh edit is a new save, and gets the short delay again
    setText(value);
    writeLocal(LOCAL_KEY, value);
    setState('unsaved');
    schedule(DEBOUNCE_MS);
  };

  useEffect(() => {
    let alive = true;
    let answered = false;
    const load = () => {
      // With nothing unsent the server is the truth — including when it answers "nothing stored", which
      // is what a cleared box leaves there, so a browser still holding the old copy drops it (#738
      // review). With something unsent this browser has the newer text: never adopt the stored note
      // over it, push it instead.
      const unsent = readLocal(LOCAL_DIRTY) === '1';
      request<DashboardNotes | null>('/notes').then(stored => {
        answered = true;
        if (!alive || touched.current) return;   // the operator typed while this was in flight
        if (unsent) { void save(); return; }
        const text = stored ? stored.text : '';
        latest.current = text;
        setText(text);
        writeLocal(LOCAL_KEY, text);
        setState(stored ? 'saved' : 'idle');
      }).catch(() => {
        // The endpoint is down: this browser's copy renders, but unsent text is not on the server, so
        // the panel says failed and keeps trying rather than looking as if all were well.
        if (alive && unsent) void save();
      });
    };
    load();
    // Asked again once a session exists, if the first request got no answer. The panel mounts before
    // the login, so with a token set that request is refused; left at that, the box stayed empty
    // after the login and the first edit replaced the stored note. Unsent text is on its own retry
    // schedule and is not asked about again.
    const again = () => { if (!answered && readLocal(LOCAL_DIRTY) !== '1') load(); };
    window.addEventListener(SESSION_EVENT, again);
    return () => { alive = false; window.removeEventListener(SESSION_EVENT, again); };
  }, []);

  // The last keystrokes before the page goes: a plain fetch is cancelled by the navigation, and a
  // keepalive one is not. Whatever happens to it, the browser copy above still holds the text.
  useEffect(() => {
    const flush = () => { if (dirty.current) void request<DashboardNotes>('/notes', { text: latest.current }, true).catch(() => {}); };
    window.addEventListener('pagehide', flush);
    return () => window.removeEventListener('pagehide', flush);
  }, []);

  return <Panel title="Notes" icon={<NotebookPen size={15}/>} aside={<span className="notes-state" data-state={state} role="status">{STATUS[state]}</span>}>
    <textarea className="notes-input" aria-label="Operator notes" placeholder="Operator notes — jot down anything worth remembering. This box saves itself; nothing here reaches the tables." spellCheck={false}
      maxLength={NOTES_MAX_CHARS} value={text} onChange={event => edit(event.target.value)} onBlur={() => { if (dirty.current) void save(); }}/>
    <p className="notes-foot">Your own scratchpad, saved on the server as you type, with this browser's copy as the fallback when it cannot be reached. <span className="count">{format(text.length)} / {format(NOTES_MAX_CHARS)}</span></p>
  </Panel>;
}
