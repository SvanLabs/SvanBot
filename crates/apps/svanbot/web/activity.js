// The activity log and the highlights (V11-44). The log is what changed between two fleets the
// stream sent, told as the operator would want it: kept by the page while it is open, newest
// first. The highlights read a bot's latest counted hands. Winnings are in big blinds, from the
// ledger's thousandths, with the luck of the bot's own draws taken out.

/** A table's short name: the last block of its id. */
function short(id) {
  return String(id).split("-").at(-1);
}

function event(bot, at, tone, text) {
  return { bot, at, tone, text };
}

/** What changed for one bot between its two statuses. */
function botChanges(was, now, at) {
  const told = [];
  const say = (tone, text) => told.push(event(now.bot, at, tone, text));
  if (!was) {
    say("ok", "joined the fleet");
    return told;
  }
  if (!was.connected && now.connected) {
    say("ok", "came online");
  }
  const table = now.table?.id ?? null;
  if (now.connected && table !== null && table !== (was.table?.id ?? null)) {
    say("", `sat at table ${short(table)}`);
  }
  // The hands counted since: the newest of the latest, one event a hand.
  const counted = (now.play?.counted ?? 0) - (was.play?.counted ?? 0);
  const recent = now.play?.recent ?? [];
  for (const mbb of counted > 0 ? recent.slice(-Math.min(counted, recent.length)) : []) {
    const bb = Math.abs(mbb / 1_000).toFixed(1);
    say(mbb >= 0 ? "up" : "down", `${mbb >= 0 ? "won" : "lost"} ${bb} bb`);
  }
  if (was.connected && !now.connected) {
    say("bad", `went offline${now.last_error ? `: ${now.last_error}` : ""}`);
  }
  if ((now.incidents ?? 0) > (was.incidents ?? 0)) {
    say("bad", "recorded an incident");
  }
  if (now.version && now.version !== was.version) {
    say("", `now runs build ${now.version}`);
  }
  const champion = now.play?.champion;
  if (champion && champion !== was.play?.champion) {
    say("", `plays champion ${champion.slice(0, 8)}`);
  }
  return told;
}

/** Each bot of ours by name, with its rank on the season board. */
function ranks(season) {
  return new Map((season?.rows ?? []).filter((row) => row.ours).map((row) => [row.name, row.rank]));
}

/**
 * The events between the fleet `before` and the fleet `after`, in the fleet's order, a bot's
 * season moves last. The first fleet has nothing to compare with, so it tells nothing.
 */
export function changes(before, after) {
  if (!before) {
    return [];
  }
  const at = after.server_ms;
  const was = new Map(before.bots.map((status) => [status.bot, status]));
  const told = after.bots.flatMap((status) => botChanges(was.get(status.bot), status, at));
  const then = ranks(before.season);
  for (const [name, rank] of ranks(after.season)) {
    const old = then.get(name);
    if (old !== undefined && old !== rank) {
      told.push(
        event(
          name,
          at,
          rank < old ? "up" : "down",
          `${rank < old ? "climbed" : "fell"} to #${rank} on the season board`,
        ),
      );
    }
  }
  return told;
}

/**
 * A bot's highlights over its latest counted hands: the best and the worst hand in big blinds,
 * the streak it is on, and its form over them in bb/100. None when it has counted no hand.
 */
export function highlights(play) {
  const recent = play?.recent ?? [];
  if (recent.length === 0) {
    return null;
  }
  const bb = recent.map((mbb) => mbb / 1_000);
  const won = bb.at(-1) >= 0;
  let length = 0;
  for (let at = bb.length - 1; at >= 0 && bb[at] >= 0 === won; at -= 1) {
    length += 1;
  }
  return {
    hands: bb.length,
    best: Math.max(...bb),
    worst: Math.min(...bb),
    streak: { won, length },
    form: (bb.reduce((sum, value) => sum + value, 0) * 100) / bb.length,
  };
}
