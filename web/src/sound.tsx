import { useEffect, useRef, useState } from 'react';
import { Volume2, VolumeX } from 'lucide-react';
import { readLocal, writeLocal } from './storage';

/** Table sounds (0181), synthesised with WebAudio so there are no asset files. Off by default; the
 * header toggle is remembered per browser. The selected bot's table plays card flicks, chips on bets
 * and raises, a thump on all-ins and a chime when we win; a big win on any bot plays a fanfare. */

const KEY = 'sv-sound';
const BIG_WIN_BB = 100;

type Tone = { freq: number; at: number; len: number; type?: OscillatorType; gain?: number; slide?: number };

let ctx: AudioContext | null = null;
function audio(): AudioContext | null {
  try {
    ctx ??= new AudioContext();
    if (ctx.state === 'suspended') void ctx.resume();
    return ctx;
  } catch { return null; }
}

function tones(list: Tone[]) {
  const a = audio();
  if (!a) return;
  const t0 = a.currentTime + 0.01;
  for (const t of list) {
    const osc = a.createOscillator(), g = a.createGain();
    osc.type = t.type ?? 'sine';
    osc.frequency.setValueAtTime(t.freq, t0 + t.at);
    if (t.slide) osc.frequency.exponentialRampToValueAtTime(t.slide, t0 + t.at + t.len);
    g.gain.setValueAtTime(0.0001, t0 + t.at);
    g.gain.exponentialRampToValueAtTime(t.gain ?? 0.12, t0 + t.at + 0.008);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + t.at + t.len);
    osc.connect(g).connect(a.destination);
    osc.start(t0 + t.at);
    osc.stop(t0 + t.at + t.len + 0.02);
  }
}

function noise(len: number, gain: number, cutoff: number) {
  const a = audio();
  if (!a) return;
  const buf = a.createBuffer(1, Math.floor(a.sampleRate * len), a.sampleRate);
  const d = buf.getChannelData(0);
  for (let i = 0; i < d.length; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / d.length) ** 3;
  const src = a.createBufferSource(), f = a.createBiquadFilter(), g = a.createGain();
  src.buffer = buf; f.type = 'bandpass'; f.frequency.value = cutoff; g.gain.value = gain;
  src.connect(f).connect(g).connect(a.destination);
  src.start();
}

export const sounds = {
  card: () => noise(0.07, 0.35, 3200),
  chips: () => { noise(0.05, 0.4, 5200); tones([{ freq: 2400, at: 0.03, len: 0.05, type: 'triangle', gain: 0.05 }, { freq: 2900, at: 0.07, len: 0.05, type: 'triangle', gain: 0.04 }]); },
  allIn: () => { tones([{ freq: 140, at: 0, len: 0.35, type: 'sawtooth', gain: 0.08, slide: 60 }]); noise(0.25, 0.3, 400); },
  win: () => tones([{ freq: 660, at: 0, len: 0.18 }, { freq: 880, at: 0.09, len: 0.25 }]),
  bigWin: () => tones([523, 659, 784, 1047].map((freq, i) => ({ freq, at: i * 0.11, len: i === 3 ? 0.6 : 0.2, type: 'triangle' as OscillatorType, gain: 0.1 }))),
};

function stored(): boolean {
  return readLocal(KEY) === 'on';
}

interface LiveEvent { type: string; slot: number; action?: string; net?: number | null; bot?: string }

/** Header toggle plus the event listener; renders only the button. */
export function SoundToggle({ selectedSlot, bigBlind }: { selectedSlot?: number; bigBlind: number }) {
  const [on, setOn] = useState(stored);
  const slot = useRef(selectedSlot);
  const bb = useRef(bigBlind);
  slot.current = selectedSlot;
  bb.current = bigBlind || 20;
  useEffect(() => {
    if (!on) return;
    const onEvent = (e: Event) => {
      const ev = (e as CustomEvent<LiveEvent>).detail;
      if (document.hidden) return;
      if (ev.type === 'result' && ev.net != null && ev.net >= BIG_WIN_BB * bb.current) { sounds.bigWin(); return; }
      if (ev.slot !== slot.current) return;
      if (ev.type === 'board') sounds.card();
      else if (ev.type === 'action' && ev.action === 'all_in') sounds.allIn();
      else if (ev.type === 'action' && (ev.action === 'raise' || ev.action === 'bet' || ev.action === 'call')) sounds.chips();
      else if (ev.type === 'result' && (ev.net ?? 0) > 0) sounds.win();
    };
    window.addEventListener('sv-live', onEvent);
    return () => window.removeEventListener('sv-live', onEvent);
  }, [on]);
  const toggle = () => {
    const next = !on;
    setOn(next);
    writeLocal(KEY, next ? 'on' : 'off');
    if (next) sounds.win(); // the click unlocks audio and previews it
  };
  return <button className={`icon-button sound-toggle ${on ? 'on' : ''}`} title={on ? 'Sound on' : 'Sound off'} aria-label={on ? 'Turn table sounds off' : 'Turn table sounds on'} aria-pressed={on} onClick={toggle}>
    {on ? <Volume2 size={17}/> : <VolumeX size={17}/>}
  </button>;
}
