import { useEffect, useState } from 'react';
import { ArrowDown, ArrowUp, Check, KeyRound, Plus, Trash2 } from 'lucide-react';
import type { SetupState } from './types';
import { request } from './api';

/** One editable bot row: an existing slot keeps its stored key unless a new one is pasted. */
interface Row { id: number; name: string; key: string; fromSlot: number | null; keyHint: string | null; enabled: boolean; check: string | null; serverName: string | null }

const call = request;

let nextId = 1;

export function SetupPage() {
  const [state, setState] = useState<SetupState | null>(null);
  const [rows, setRows] = useState<Row[]>([]);
  const [buyIn, setBuyIn] = useState(5000);
  const [seek, setSeek] = useState(30);
  const [message, setMessage] = useState<{kind: 'ok' | 'error'; text: string} | null>(null);
  const [busy, setBusy] = useState(false);

  const load = () => call<SetupState>('/setup').then(s => {
    setState(s);
    setRows(s.bots.map(b => ({ id: nextId++, name: b.name, key: '', fromSlot: b.slot, keyHint: b.key_hint, enabled: b.enabled, check: null, serverName: null })));
    setBuyIn(s.buy_in);
    setSeek(s.seek_top_rank);
  }).catch(e => setMessage({kind: 'error', text: (e as Error).message}));
  useEffect(() => { load(); }, []);

  const update = (id: number, patch: Partial<Row>) => setRows(rs => rs.map(r => r.id === id ? {...r, ...patch} : r));
  const move = (index: number, by: number) => setRows(rs => { const next = [...rs]; const [row] = next.splice(index, 1); next.splice(index + by, 0, row); return next; });
  const verify = async (row: Row) => {
    update(row.id, {check: 'Checking…', serverName: null});
    try {
      const r = await call<{name: string | null; pro_tier: boolean | null}>('/setup/verify-key', row.key.trim() ? {key: row.key.trim()} : {slot: row.fromSlot});
      update(row.id, {check: `Key accepted${r.name ? ` · registered as ${r.name}` : ''}${r.pro_tier == null ? '' : r.pro_tier ? ' · Pro' : ' · Free (one bot only)'}`, serverName: r.name});
    } catch (e) {
      update(row.id, {check: (e as Error).message, serverName: null});
    }
  };
  const save = async () => {
    setBusy(true); setMessage(null);
    try {
      const body = { buy_in: buyIn, seek_top_rank: seek, bots: rows.map(r => ({ name: r.name.trim(), enabled: r.enabled, ...(r.key.trim() ? {key: r.key.trim()} : {from_slot: r.fromSlot}) })) };
      const r = await call<{restart: string}>('/setup', body);
      setMessage({kind: 'ok', text: r.restart === 'scheduled'
        ? 'Saved. The fleet restarts at the next moment no bot is mid-turn (under a minute) and every seat resyncs.'
        : 'Saved to .env. Restart the fleet (scripts/restart-bot.sh) to apply it.'});
      await load();
    } catch (e) {
      setMessage({kind: 'error', text: (e as Error).message});
    } finally { setBusy(false); }
  };

  const max = state?.max_bots ?? 5;
  return <main className="help-page setup-page" aria-label="Bot setup">
    <a className="text-button" href="#">← Back to control room</a>
    <span className="eyebrow">SVANBOT 11 · BOT SETUP</span><h1>Bots and table settings</h1>
    <p className="help-intro">Add, rename, reorder, switch off or remove bots and set the buy-in. Changes are saved to the server's <code>.env</code> (kept private, previous version kept). API keys are never shown again after saving — only their last four characters.</p>
    {state?.write_blocked && <div className="error-banner" role="alert">{state.write_blocked}</div>}
    {state?.restart_pending && <div className="connection-banner">A saved change is waiting for the fleet restart.</div>}
    <section aria-label="Bots">
      <h2>Bots <span className="count">{rows.length}/{max}</span></h2>
      {rows.map((row, index) => <div className="setup-bot" key={row.id}>
        <div className="setup-bot-order">
          <button className="icon-button" aria-label={`Move ${row.name || 'bot'} up`} disabled={index === 0} onClick={() => move(index, -1)}><ArrowUp size={14}/></button>
          <button className="icon-button" aria-label={`Move ${row.name || 'bot'} down`} disabled={index === rows.length - 1} onClick={() => move(index, 1)}><ArrowDown size={14}/></button>
        </div>
        <label>Name<input aria-label={`Bot ${index + 1} name`} value={row.name} maxLength={32} onChange={e => update(row.id, {name: e.target.value})}/></label>
        <label>API key<input aria-label={`Bot ${index + 1} API key`} type="password" autoComplete="off" placeholder={row.keyHint ? `Stored key ${row.keyHint} (leave empty to keep)` : 'Paste the Self Host API key'} value={row.key} onChange={e => update(row.id, {key: e.target.value, check: null, serverName: null})}/></label>
        <div className="setup-bot-actions">
          <button className="button" disabled={!row.key.trim() && row.fromSlot == null} onClick={() => verify(row)}><KeyRound size={13}/>Check key</button>
          <button className={`toggle ${row.enabled ? 'on' : ''}`} role="switch" aria-checked={row.enabled} aria-label={`${row.name || 'Bot'} plays`} onClick={() => update(row.id, {enabled: !row.enabled})}><i/></button>
          <button className="icon-button" aria-label={`Remove ${row.name || 'bot'}`} onClick={() => setRows(rs => rs.filter(r => r.id !== row.id))}><Trash2 size={14}/></button>
        </div>
        {row.check && <p className="setup-check">{row.check}{row.serverName && row.serverName !== row.name.trim() && <> <button className="text-button" onClick={() => update(row.id, {name: row.serverName!})}><Check size={12}/>Use {row.serverName}</button></>}</p>}
      </div>)}
      <button className="text-button" disabled={rows.length >= max} onClick={() => setRows(rs => [...rs, {id: nextId++, name: '', key: '', fromSlot: null, keyHint: null, enabled: true, check: null, serverName: null}])}><Plus size={13}/>Add bot{rows.length >= max ? ` (limit ${max})` : ''}</button>
    </section>
    <section aria-label="Table settings">
      <h2>Table settings</h2>
      <label className="setup-field">Maximum buy-in (chips)<input aria-label="Maximum buy-in" type="number" min={1000} max={5000} step={100} value={buyIn} onChange={e => setBuyIn(Number(e.target.value))}/></label>
      <label className="setup-field">Seek tables with a top-N bot (0 = off)<input aria-label="Seek top rank" type="number" min={0} max={1000} value={seek} onChange={e => setSeek(Number(e.target.value))}/></label>
    </section>
    {message && <div className={message.kind === 'ok' ? 'connection-banner' : 'error-banner'} role={message.kind === 'ok' ? 'status' : 'alert'}>{message.text}</div>}
    <div className="setup-save">
      <button className="button" disabled={busy || !state?.can_write} onClick={save}>{busy ? 'Saving…' : 'Save and apply'}</button>
      <span>{state?.supervised ? 'Applies with a short fleet restart between turns.' : 'Applies on the next fleet restart.'}</span>
    </div>
  </main>;
}
