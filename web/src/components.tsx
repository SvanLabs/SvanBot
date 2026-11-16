// The status-first page's pieces (V11-11): one sticky strip that says what the fleet is doing and
// what needs attention, then one card per bot. Pure props in, markup out, so the page is tested
// with `renderToStaticMarkup`.

import type { ReactNode } from "react";
import {
  type BotStatus,
  botView,
  duration,
  fleetProblems,
  freshness,
  isStale,
  type Problem,
  type Tone,
} from "./fleet";
import { serverNow, type View } from "./view";

export interface StripFreshness {
  tone: Tone;
  label: string;
}

function summary(bots: BotStatus[], nowMs: number): string {
  const silent = bots.filter((bot) => isStale(bot, nowMs)).length;
  const playing = bots.filter(
    (bot) => !isStale(bot, nowMs) && bot.connected && bot.table !== null,
  ).length;
  const waiting = bots.filter(
    (bot) => !isStale(bot, nowMs) && bot.connected && bot.table === null,
  ).length;
  const offline = bots.length - silent - playing - waiting;
  const parts = [
    `${bots.length} ${bots.length === 1 ? "bot" : "bots"}`,
    `${playing} playing`,
    `${waiting} waiting`,
  ];
  if (silent > 0) {
    parts.push(`${silent} not reporting`);
  }
  if (offline > 0) {
    parts.push(`${offline} offline`);
  }
  return parts.join(" · ");
}

/** The sticky strip: fleet count, view freshness, and the one problem that matters most. */
export function StatusStrip({
  bots,
  problems,
  fresh,
  nowMs,
}: {
  bots: BotStatus[];
  problems: Problem[];
  fresh: StripFreshness;
  nowMs: number;
}) {
  const worst = problems[0];
  return (
    <header className="strip">
      <h1 className="brand">
        SvanBot <span className="brand-sub">fleet</span>
      </h1>
      <p className="summary">{summary(bots, nowMs)}</p>
      <span className={`pill tone-${fresh.tone}`}>{fresh.label}</span>
      <p className={worst === undefined ? "clean" : "worst"} role="status">
        {worst === undefined ? (
          "All quiet"
        ) : (
          <>
            <span className={`dot tone-${worst.tone}`} aria-hidden="true" />
            {worst.text}
            {problems.length > 1 ? (
              <span className="more"> +{problems.length - 1} more</span>
            ) : null}
          </>
        )}
      </p>
    </header>
  );
}

function seats(table: NonNullable<BotStatus["table"]>): ReactNode {
  if (table.players.length === 0) {
    return <p className="quiet">No seats reported yet</p>;
  }
  return (
    <ul className="seats">
      {table.players.map((player) => (
        <li key={player.seat} className={player.folded ? "seat folded" : "seat"}>
          <span className="seat-number">{player.seat}</span>
          <span className="seat-name">{player.name}</span>
          <span className="seat-stack">{player.stack.toLocaleString("en-US")}</span>
          {player.seat === table.seat ? <span className="tag you">you</span> : null}
          {player.folded ? <span className="tag">folded</span> : null}
        </li>
      ))}
    </ul>
  );
}

function tableBlock(table: NonNullable<BotStatus["table"]>): ReactNode {
  return (
    <section className="table" aria-label="table">
      <p className="line">
        <span className="street">{table.street ?? "between hands"}</span>
        <span>pot {table.pot}</span>
        <span>big blind {table.big_blind}</span>
        <span>dealer seat {table.dealer}</span>
        {table.hand_id === null ? null : <span>hand {table.hand_id}</span>}
      </p>
      <p className="board">{table.board.length === 0 ? "no board yet" : table.board.join(" ")}</p>
      {seats(table)}
    </section>
  );
}

/** One bot: what it is doing, how long it has been up, and the table it sits at. */
export function BotCard({ status, nowMs }: { status: BotStatus; nowMs: number }) {
  const view = botView(status, nowMs);
  return (
    <article className={`card tone-${view.tone}`} data-bot={status.bot}>
      <header className="card-head">
        <h2>{status.bot}</h2>
        <span className={`pill tone-${view.tone}`}>{view.label}</span>
      </header>
      <p className="detail">{view.detail}</p>
      <ul className="stats">
        <li>{status.hands} hands</li>
        <li>
          {status.incidents} incidents
          {status.run_incidents > 0 ? ` (${status.run_incidents} this run)` : ""}
        </li>
        <li>
          {status.last_hand_ms === null
            ? "no hand yet"
            : `last hand ${duration(nowMs - status.last_hand_ms)} ago`}
        </li>
        <li>up {duration(nowMs - status.started_ms)}</li>
        <li>pid {status.pid}</li>
        <li>{status.version}</li>
      </ul>
      {status.table === null ? null : tableBlock(status.table)}
    </article>
  );
}

/** What the page says before any bot has written a status. */
export function EmptyFleet() {
  return (
    <section className="empty">
      <h2>No bot has reported yet</h2>
      <p>
        Each bot writes a status beside its store. Point the dashboard at that directory with{" "}
        <code>SVANBOT_DATA</code>, then start a bot.
      </p>
    </section>
  );
}

/** The whole page: the strip and either one card per bot or the empty state. */
export function FleetView({ view, localNow }: { view: View; localNow: number }) {
  const now = serverNow(view, localNow) ?? 0;
  const fleet = view.fleet ?? { server_ms: 0, bots: [], problems: [] };
  const fresh = freshness(view.serverMs, now);
  const problems = fleetProblems(fleet, now, view.error);
  return (
    <div className="fleet">
      <StatusStrip bots={fleet.bots} problems={problems} fresh={fresh} nowMs={now} />
      <main className="content">
        {problems.length > 1 ? (
          <section className="problems" aria-label="other problems">
            <ul>
              {problems.slice(1).map((problem) => (
                <li key={problem.text} className={`tone-${problem.tone}`}>
                  {problem.text}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        {fleet.bots.length === 0 ? (
          <EmptyFleet />
        ) : (
          <div className="cards">
            {fleet.bots.map((status) => (
              <BotCard key={status.bot} status={status} nowMs={now} />
            ))}
          </div>
        )}
      </main>
    </div>
  );
}
