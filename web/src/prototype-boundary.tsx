// PROTOTYPE — throwaway (#704). Answers one question: what should the operator SEE
// during the season boundary so defending #1 never depends on logs? Three structurally
// different variants, mounted in the Season race panel, switchable with ?variant=A|B|C
// and the floating bar. Stub data stands in for the fields the ticket proposes
// (clock -> wind-down/freeze, cooldown-due-in, session/day-anchored deltas); nothing in
// production reads this file. Branch: prototype-boundary-704. Delete once #704 decides.

import React, { useEffect, useState } from 'react';
import { fmt, sgn } from './format';

// Season 13 end, from GET /season/current (the live clock the fleet already holds).
const SEASON_END = Date.parse('2026-10-04T11:46:00Z');
const FREEZE_BEFORE = 15 * 60 * 1000; // season.rs FREEZE_BEFORE_END
const WIND_DOWN = 5 * 60 * 1000; // season.rs WIND_DOWN

interface Seat { name: string; rank: number; score: number; cooldown: number | null; day: number; session: number }
// Stub standings, 2026-10-03. day/session deltas are the *proposed* anchors: score change
// since 00:00Z and since this process's session start — not the 300s frontier of today.
const SEATS: Seat[] = [
  { name: 'SvanBotV10', rank: 1, score: 1832170, cooldown: null, day: 32140, session: 12890 },
  { name: 'Svanism', rank: 2, score: 1670673, cooldown: 192, day: 18430, session: -4020 },
  { name: 'SurSvan', rank: 3, score: 1536983, cooldown: null, day: 9870, session: 3110 },
  { name: 'SuraGunnar', rank: 4, score: 1500847, cooldown: 47, day: -2310, session: -7740 },
  { name: 'Svanar', rank: 5, score: 1429866, cooldown: null, day: 7640, session: 5120 },
];
const RIVAL = { name: 'Quietflute', score: 1030433 };

function useTick() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(id); }, []);
  return now;
}

/** The same state season.rs computes: phase, moves allowed, and the two countdown targets. */
function boundary(now: number) {
  const toEnd = SEASON_END - now;
  const toFreeze = toEnd - FREEZE_BEFORE;
  const phase = toEnd <= 0 ? 'over' : toEnd <= WIND_DOWN ? 'wind-down' : toFreeze <= 0 ? 'frozen' : 'open';
  return { toEnd, toFreeze, phase, moves: phase === 'open' };
}

function clock(ms: number) {
  if (ms <= 0) return '0:00';
  const s = Math.floor(ms / 1000);
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return d > 0 ? `${d}d ${h}h ${m}m` : h > 0 ? `${h}h ${m}m ${sec}s` : `${m}:${String(sec).padStart(2, '0')}`;
}
const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
const utc = (ms: number) => new Date(ms).toISOString().slice(11, 16) + 'Z';

const chip = (text: string, tone: string) => <span style={{ padding: '2px 8px', borderRadius: 4, fontSize: 11, letterSpacing: '0.06em', fontWeight: 600, background: tone, color: '#0b0e13' }}>{text}</span>;
const mono: React.CSSProperties = { fontFamily: 'IBM Plex Mono, monospace' };
const dim: React.CSSProperties = { opacity: 0.65, fontSize: 11 };

function PhaseChip({ b }: { b: ReturnType<typeof boundary> }) {
  const tone = b.phase === 'open' ? '#6daa98' : b.phase === 'frozen' ? '#e4b956' : '#b68578';
  return chip(b.phase === 'open' ? 'OPEN' : b.phase === 'frozen' ? 'FREEZE — NO MOVES' : b.phase === 'wind-down' ? 'WIND-DOWN' : 'ENDED', tone);
}

/** A — Boundary strip: one glanceable line above the race hero; nothing to open. */
function VariantA() {
  const now = useTick();
  const b = boundary(now);
  const cooling = SEATS.filter(s => s.cooldown != null);
  return <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap', padding: '8px 10px', border: '1px solid #2a3140', borderRadius: 6, marginBottom: 8, fontSize: 12 }}>
    <PhaseChip b={b} />
    <b style={mono}>{b.phase === 'frozen' ? `END IN ${clock(b.toEnd)}` : `FREEZE IN ${clock(b.toFreeze)}`}</b>
    <span style={dim}>freeze {utc(SEASON_END - FREEZE_BEFORE)} · wind-down {utc(SEASON_END - WIND_DOWN)} · end {utc(SEASON_END)}</span>
    <span style={{ flex: 1 }} />
    {cooling.map(s => <span key={s.name} style={{ ...mono, color: '#e4b956' }} title="auto-rebuy cooldown; seat is not at a table">{s.name} rebuy {mmss(s.cooldown!)}</span>)}
    <span style={mono}>#1 SvanBotV10 {sgn(SEATS[0].day)} today · gap to {RIVAL.name} <b>{fmt(SEATS[0].score - RIVAL.score)}</b></span>
  </div>;
}

/** B — Boundary timeline: the boundary as a time axis, cooldowns as ticks, anchors as a card. */
function VariantB() {
  const now = useTick();
  const b = boundary(now);
  const start = SEASON_END - 24 * 3600 * 1000; // a 24h window ending at the boundary
  const span = SEASON_END + 10 * 60 * 1000 - start;
  const at = (ms: number) => `${Math.max(0, Math.min(100, ((ms - start) / span) * 100))}%`;
  const best = SEATS[0];
  return <div style={{ border: '1px solid #2a3140', borderRadius: 6, padding: 10, marginBottom: 8 }}>
    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
      <span className="eyebrow">BOUNDARY TIMELINE</span><PhaseChip b={b} />
    </div>
    <div style={{ position: 'relative', height: 44, background: 'linear-gradient(90deg,#1a2030,#141a26)' }}>
      <div style={{ position: 'absolute', left: at(SEASON_END - WIND_DOWN), right: 0, top: 0, bottom: 0, background: 'rgba(182,133,120,0.25)' }} />
      <div style={{ position: 'absolute', left: at(SEASON_END - FREEZE_BEFORE), top: 0, bottom: 0, width: 2, background: '#e4b956' }} />
      <div style={{ position: 'absolute', left: at(now), top: 0, bottom: 0, width: 2, background: '#8fa2d8' }}>
        <span style={{ ...mono, position: 'absolute', top: -2, left: 6, fontSize: 10, whiteSpace: 'nowrap' }}>now {utc(now)}</span>
      </div>
      {SEATS.filter(s => s.cooldown != null).map((s, i) => <span key={s.name} title={`${s.name} rebuy due`} style={{ position: 'absolute', left: at(now + s.cooldown! * 1000), bottom: 4 + i * 16, ...mono, fontSize: 10, color: '#e4b956', whiteSpace: 'nowrap' }}>▪ {s.name} {mmss(s.cooldown!)}</span>)}
    </div>
    <div style={{ ...mono, display: 'flex', justifyContent: 'space-between', fontSize: 10, ...dim }}>
      <span>{utc(start)}</span><span>freeze {utc(SEASON_END - FREEZE_BEFORE)}</span><span>wind-down {utc(SEASON_END - WIND_DOWN)}</span><span>end {utc(SEASON_END)}</span>
    </div>
    <div style={{ display: 'flex', gap: 24, marginTop: 8 }}>
      <div><span style={dim}>#1 {best.name}</span><br /><b style={mono}>{fmt(best.score)}</b></div>
      <div><span style={dim}>today (00:00Z anchor)</span><br /><b style={{ ...mono, color: best.day >= 0 ? '#6daa98' : '#b68578' }}>{sgn(best.day)}</b></div>
      <div><span style={dim}>this session (seat anchor)</span><br /><b style={{ ...mono, color: best.session >= 0 ? '#6daa98' : '#b68578' }}>{sgn(best.session)}</b></div>
      <div><span style={dim}>{b.phase === 'open' ? 'freeze in' : 'end in'}</span><br /><b style={mono}>{clock(b.phase === 'open' ? b.toFreeze : b.toEnd)}</b></div>
    </div>
  </div>;
}

/** C — Ops console: per-seat state you can audit line by line; alert first, detail under. */
function VariantC() {
  const now = useTick();
  const b = boundary(now);
  const cooling = SEATS.filter(s => s.cooldown != null);
  const row: React.CSSProperties = { display: 'grid', gridTemplateColumns: '1.4fr 0.5fr 0.8fr 0.8fr 0.8fr 0.8fr 1fr', gap: 6, padding: '3px 6px', borderTop: '1px solid #232a38', alignItems: 'center' };
  return <div style={{ border: '1px solid #2a3140', borderRadius: 6, marginBottom: 8 }}>
    <div style={{ padding: '6px 8px', background: b.moves ? 'rgba(109,170,152,0.12)' : 'rgba(228,185,86,0.16)', fontSize: 12 }}>
      <b>{b.phase === 'open' ? 'NOMINAL' : b.phase === 'frozen' ? 'MOVES FROZEN' : 'WIND-DOWN'}</b>
      {' · '}{b.moves ? 'table moves allowed' : 'no voluntary table moves'}
      {' · '}{cooling.length} seat{cooling.length === 1 ? '' : 's'} in rebuy cooldown{cooling.length ? ` (${cooling.map(s => `${s.name} ${mmss(s.cooldown!)}`).join(', ')})` : ''}
      {' · '}freeze {b.phase === 'open' ? 'in ' + clock(b.toFreeze) : 'passed'} · end in {clock(b.toEnd)}
    </div>
    <div style={{ ...row, ...dim }}><span>seat</span><span>rank</span><span>state</span><span>session Δ</span><span>day Δ</span><span>score</span><span>cooldown due</span></div>
    {SEATS.map(s => <div key={s.name} style={row}>
      <span>{s.name}{s.rank === 1 && <em style={{ marginLeft: 6, fontSize: 10 }}>#1</em>}</span>
      <span style={mono}>#{s.rank}</span>
      <span style={{ color: s.cooldown != null ? '#e4b956' : '#6daa98' }}>{s.cooldown != null ? 'rebuy' : b.moves ? 'playing' : 'frozen'}</span>
      <span style={{ ...mono, color: s.session >= 0 ? '#6daa98' : '#b68578' }}>{sgn(s.session)}</span>
      <span style={{ ...mono, color: s.day >= 0 ? '#6daa98' : '#b68578' }}>{sgn(s.day)}</span>
      <span style={mono}>{fmt(s.score)}</span>
      <span style={mono}>{s.cooldown != null ? mmss(s.cooldown) : '—'}</span>
    </div>)}
    <div style={{ ...row, ...dim }}><span>rival {RIVAL.name}</span><span>—</span><span>—</span><span>—</span><span>—</span><span>{fmt(RIVAL.score)}</span><span>gap {fmt(SEATS[0].score - RIVAL.score)}</span></div>
  </div>;
}

const VARIANTS = [['A', 'Boundary strip', VariantA], ['B', 'Boundary timeline', VariantB], ['C', 'Ops console', VariantC]] as const;

/** Mounted inside the Season race panel; the bar is the prototype's only chrome. */
export function BoundaryPrototype() {
  const [v, setV] = useState(() => new URLSearchParams(location.search).get('variant') ?? 'A');
  const go = (step: number) => setV(cur => {
    const i = VARIANTS.findIndex(([k]) => k === cur);
    const next = VARIANTS[(i + step + VARIANTS.length) % VARIANTS.length][0];
    const url = new URL(location.href);
    url.searchParams.set('variant', next);
    history.replaceState(null, '', url);
    return next;
  });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (['INPUT', 'TEXTAREA'].includes(t.tagName) || t.isContentEditable) return;
      if (e.key === 'ArrowLeft') go(-1);
      if (e.key === 'ArrowRight') go(1);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const [key, label, Body] = VARIANTS.find(([k]) => k === v) ?? VARIANTS[0];
  return <>
    <Body />
    {import.meta.env.DEV && <div style={{ position: 'fixed', bottom: 16, left: '50%', transform: 'translateX(-50%)', zIndex: 999, display: 'flex', alignItems: 'center', gap: 10, background: '#0b0e13', border: '1px solid #8fa2d8', borderRadius: 999, padding: '6px 14px', boxShadow: '0 4px 18px rgba(0,0,0,0.5)', fontSize: 12 }}>
      <button className="text-button" onClick={() => go(-1)} aria-label="Previous variant">←</button>
      <span style={mono}>PROTOTYPE #704 · {key} — {label} · ?variant=</span>
      <button className="text-button" onClick={() => go(1)} aria-label="Next variant">→</button>
    </div>}
  </>;
}
