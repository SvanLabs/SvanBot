/** Hand replay and the starting-hand guide. */
import { useEffect, useState } from 'react';
import { Gauge, Pause as PauseIcon, ChevronLeft, ChevronRight, Play, Spade, X } from 'lucide-react';
import { useAutoPlay } from './fun';
import type { Hand, ReplayEvent } from './types';
import { format, Card, Panel } from './ui';
import { StaleNote, usePoll } from './api';
import { time } from './format';
import { PlayerName } from './playername';

export function Replay({events, hand, onClose}: {events:ReplayEvent[];hand:Hand;onClose:()=>void}) {
  const [step,setStep] = useState(0);
  const auto = useAutoPlay(events.length, step, setStep);
  useEffect(() => {
    // Escape closes the replay; the scout view below it checks for this modal before closing itself.
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const shown = events.slice(0,step+1);
  const latest = shown[shown.length-1];
  let board:string[] = [];
  for(const event of shown) {
    if(event.type === 'community_cards' && Array.isArray(event.data.cards)) board = event.data.cards.length >= 3 ? event.data.cards as string[] : [...board,...event.data.cards as string[]];
    if(Array.isArray(event.data.board)) board = event.data.board as string[];
  }
  return <div className="modal-backdrop" onClick={onClose}><section className="modal replay-modal" role="dialog" aria-modal="true" aria-label="Hand replay" onClick={event => event.stopPropagation()}><div className="modal-heading"><div><span className="eyebrow">HAND REPLAY</span><h2>{hand.hand_id}</h2></div><button className="icon-button" aria-label="Close replay" onClick={onClose}><X/></button></div><div className="replay-board"><div><Card card={hand.hole[0]}/><Card card={hand.hole[1]}/></div><span className="replay-divider"/>{Array.from({length:5},(_,index)=><Card key={index} card={board[index]}/>)}</div><div className="replay-controls"><button className="button" disabled={step === 0} onClick={()=>setStep(step-1)}><ChevronLeft size={15}/></button><input aria-label="Replay position" type="range" min="0" max={Math.max(0,events.length-1)} value={step} onChange={event=>setStep(Number(event.target.value))}/><button className="button" disabled={step >= events.length-1} onClick={()=>setStep(step+1)}><ChevronRight size={15}/></button><button className="button" aria-label={auto.playing ? 'Pause replay' : 'Play replay'} onClick={()=>{ if(step >= events.length-1) setStep(0); auto.setPlaying(!auto.playing); }}>{auto.playing ? <PauseIcon size={14}/> : <Play size={14}/>}</button><button className="button replay-speed" aria-label="Replay speed" onClick={()=>auto.setSpeed(auto.speed >= 4 ? 1 : auto.speed * 2)}><Gauge size={13}/>{auto.speed}x</button></div><div className="replay-current"><span className="tag">{latest?.type || 'No events'}</span><span>{step+1} / {events.length}</span></div><div className="replay-event-list">{shown.map((event,index)=><div key={index}><time>{time(event.ts, { seconds: true })}</time><b>{event.type.replaceAll('_',' ')}</b><span>{event.data.name ? <PlayerName name={String(event.data.name)}/> : null} {String(event.data.action || '')} {event.data.amount ? format(Number(event.data.amount)) : ''}</span></div>)}</div></section></div>;
}

export function StartingHands({version}: {version?: string | number}) {
  const poll = usePoll<{hand:string;score:number;open:boolean[]}[]>('/starting-hands', 60000, version);
  const rows = poll.data ?? [];
  const [position,setPosition] = useState(0);
  return <Panel title="Starting hand library" icon={<Spade size={15}/>} aside={<span className="tag">169 HAND CLASSES</span>}><StaleNote poll={poll}/>{!poll.data && !poll.error && <p className="footnote">Loading the opening guide…</p>}<div className="position-tabs">{['UTG','HJ','CO','BTN','SB','BB'].map((label,index)=><button key={label} className={index===position?'active':''} onClick={()=>setPosition(index)}>{label}</button>)}</div><div className="range-grid">{rows.map(row=><span key={row.hand} className={row.open[position]?'range-open':''} title={`${row.hand}: score ${row.score.toFixed(3)} · ${row.open[position]?'in opening guide':'outside opening guide'}`}>{row.hand}</span>)}</div><p className="footnote">Six-seat first-in heuristic from the champion. Gold = opening guide. Facing raises, stack depth, pot odds and opponent ranges change the decision. This is not a solved optimal range.</p></Panel>;
}
