/** Inline SVG diagrams for the docs page (0300). `docs/GUIDE.md` keeps an ASCII version of each in a
 * ```diagram:<name> fence, readable anywhere; the page draws the same picture here and keeps the
 * ASCII one click away. Colours come from the dashboard's tokens through the `dg-*` classes. */

type Tone = 'core' | 'store' | 'side' | 'out';

function Box({ x, y, w, h, title, lines = [], tone = 'core' }: { x: number; y: number; w: number; h: number; title: string; lines?: string[]; tone?: Tone }) {
  return <g className={`dg-node dg-${tone}`}>
    <rect x={x} y={y} width={w} height={h} rx={6} />
    <text x={x + w / 2} y={y + 18} className="dg-title" textAnchor="middle">{title}</text>
    {lines.map((l, i) => <text key={i} x={x + w / 2} y={y + 34 + i * 13} className="dg-line" textAnchor="middle">{l}</text>)}
  </g>;
}

function Arrow({ d, label, lx, ly, both = false }: { d: string; label?: string; lx?: number; ly?: number; both?: boolean }) {
  return <g className="dg-edge">
    <path d={d} markerEnd="url(#dg-head)" markerStart={both ? 'url(#dg-tail)' : undefined} />
    {label && <text x={lx} y={ly} className="dg-label" textAnchor="middle">{label}</text>}
  </g>;
}

function Frame({ label, viewBox, children }: { label: string; viewBox: string; children: React.ReactNode }) {
  return <svg className="dg" viewBox={viewBox} role="img" aria-label={label}>
    <defs>
      <marker id="dg-head" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" className="dg-tip" /></marker>
      <marker id="dg-tail" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" className="dg-tip" /></marker>
    </defs>
    {children}
  </svg>;
}

/** Processes and where the data lives. */
function Processes() {
  return <Frame label="Processes and data: openpoker.ai, the fleet, the learner, the analyst and the two databases" viewBox="0 0 760 330">
    <Box x={290} y={8} w={180} h={44} title="dashboard" lines={['web/ (React), SSE + API']} tone="side" />
    <Box x={5} y={96} w={150} h={58} title="openpoker.ai" lines={['tables, leaderboard,', 'hand histories']} tone="out" />
    <Box x={265} y={88} w={230} h={74} title="sv10-bot" lines={['5 bots: seat, track, decide, act;', 'stores hands and decisions;', 'hourly backup, dashboard API']} />
    <Box x={570} y={88} w={180} h={74} title="learner" lines={['fits, clone population,', 'challenger search,', 'promotes params.v1']} />
    <Box x={290} y={206} w={180} h={58} title="svanbot10.db" lines={['hands, decisions, kv,', 'calibration, audits']} tone="store" />
    <Box x={40} y={206} w={170} h={58} title="history.db" lines={['server exports,', 'training corpus']} tone="store" />
    <Box x={570} y={206} w={180} h={58} title="analyst" lines={['deep re-solve of big', 'decisions; wiring table']} />
    <Box x={290} y={280} w={180} h={44} title="archive (nightly)" lines={['full / differential']} tone="side" />
    <Box x={570} y={280} w={180} h={44} title="/backup-disk (HDD)" lines={['daily, weekly, monthly']} tone="out" />
    <Arrow d="M155,125 L263,125" both label="WebSocket + REST" lx={209} ly={116} />
    <Arrow d="M380,88 L380,54" label="state" lx={402} ly={74} />
    <Arrow d="M380,162 L380,204" both />
    <Arrow d="M265,150 L200,204" label="exports" lx={214} ly={172} />
    <Arrow d="M568,125 L497,125" label="params.v1" lx={532} ly={117} />
    <Arrow d="M660,162 L472,212" label="reads hands" lx={702} ly={180} />
    <Arrow d="M568,235 L472,235" both label="audit queue" lx={520} ly={228} />
    <Arrow d="M380,264 L380,278" />
    <Arrow d="M470,302 L568,302" />
  </Frame>;
}

/** One decision, from the turn to the stored record. */
function DecisionPath() {
  const top = [
    { t: 'your_turn', l: ['hand_id, turn_token,', 'valid actions'] },
    { t: 'situation', l: ['seats, stacks, pot,', 'board, history'] },
    { t: 'opponent ranges', l: ['their stats, our image,', 'size and timing tells'] },
    { t: 'shared deals', l: ['Monte Carlo on every', 'core, or exact runouts'] },
  ];
  const bottom = [
    { t: 'price candidates', l: ['fold, check/call, raise', 'sizes: stats + network,', 'fold offsets, live fits'] },
    { t: 'self-calibration', l: ['add the measured bias', 'per category (bounded', 'by margin evidence)'] },
    { t: 'choose, legalize', l: ['best EV, mixing only', 'near-equal options;', 'only valid actions'] },
    { t: 'send + record', l: ['action with hand_id and', 'turn_token; decision,', 'replay, audit queue'] },
  ];
  const w = 170, gap = 20;
  return <Frame label="The decision path in eight steps" viewBox="0 0 760 250">
    {top.map((s, i) => <Box key={s.t} x={10 + i * (w + gap)} y={10} w={w} h={62} title={s.t} lines={s.l} />)}
    {top.slice(0, -1).map((_, i) => <Arrow key={i} d={`M${10 + i * (w + gap) + w},41 L${10 + (i + 1) * (w + gap) - 2},41`} />)}
    <Arrow d={`M${10 + 3 * (w + gap) + w / 2},72 L${10 + 3 * (w + gap) + w / 2},96 L${10 + w / 2},96 L${10 + w / 2},128`} />
    {bottom.map((s, i) => <Box key={s.t} x={10 + i * (w + gap)} y={130} w={w} h={74} title={s.t} lines={s.l} tone={i === 3 ? 'store' : 'core'} />)}
    {bottom.slice(0, -1).map((_, i) => <Arrow key={i} d={`M${10 + i * (w + gap) + w},167 L${10 + (i + 1) * (w + gap) - 2},167`} />)}
    <text x={380} y={232} className="dg-label" textAnchor="middle">about 0.1 s per decision (p50; p95 about 0.3 s) against a 45 s deadline; a legal check or fold is ready if anything fails</text>
  </Frame>;
}

/** The learner's loop, and the live side-channel for challengers simulation cannot settle. */
function LearnerLoop() {
  return <Frame label="The learner loop: evidence, clones, search, fresh-deal gate, promotion" viewBox="0 0 760 270">
    <Box x={10} y={20} w={200} h={58} title="new live hands" lines={['paced by the dashboard', 'cooldown and hand limit']} tone="out" />
    <Box x={280} y={20} w={200} h={58} title="evidence refresh" lines={['fold and range fits, live fits,', 'per-opponent fits, network']} />
    <Box x={550} y={20} w={200} h={58} title="clone population" lines={['profile clones of the', '16 most-seen opponents']} />
    <Box x={550} y={150} w={200} h={58} title="challenger search" lines={['successive halving on', 'identical (paired) deals']} />
    <Box x={280} y={150} w={200} h={58} title="fresh-deal gate" lines={['95% lower bound above', '+1 bb/100 on new deals']} tone="store" />
    <Box x={10} y={150} w={200} h={58} title="promote params.v1" lines={['the fleet reloads it', 'between hands']} />
    <Arrow d="M210,49 L278,49" />
    <Arrow d="M480,49 L548,49" />
    <Arrow d="M650,78 L650,148" />
    <Arrow d="M548,179 L482,179" />
    <Arrow d="M278,179 L212,179" label="pass" lx={245} ly={171} />
    <Arrow d="M110,150 L110,80" label="next cycle" lx={140} ly={118} />
    <Arrow d="M380,208 L380,238" label="not settled" lx={420} ly={226} />
    <text x={380} y={256} className="dg-label" textAnchor="middle">experiment mode: bots #4 and #5 play an unresolved challenger live against the champion (0291)</text>
  </Frame>;
}

const DIAGRAMS: Record<string, () => React.ReactElement> = { processes: Processes, 'decision-path': DecisionPath, 'learner-loop': LearnerLoop };

/** The SVG for a ```diagram:<name> fence, with its ASCII source kept one click away; an unknown name
 * shows the ASCII as it is. */
export function DocsDiagram({ name, ascii }: { name: string; ascii: string }) {
  const Svg = DIAGRAMS[name];
  if (!Svg) return <pre><code>{ascii}</code></pre>;
  return <figure className="dg-figure">
    <Svg />
    <details><summary>Text version</summary><pre><code>{ascii}</code></pre></details>
  </figure>;
}
