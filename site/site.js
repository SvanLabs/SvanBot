// Renders the Arena's pinned run from `headline.json`, which the Pages workflow copies from
// `fleet/headline.json` when it publishes: the same file the README's headline is written from.
// Without it (a local copy, say), the page keeps its note and links to the README.
(async () => {
  const board = document.getElementById("board");
  const provenance = document.getElementById("provenance");
  let run;
  try {
    const answer = await fetch("headline.json", { cache: "no-store" });
    if (!answer.ok) return;
    run = await answer.json();
  } catch {
    return;
  }
  const rows = Array.isArray(run.rows) ? run.rows : [];
  if (rows.length === 0) return;

  // Every whisker shares one scale, which always reaches zero, so a row's side of zero shows.
  const low = Math.min(0, ...rows.map((row) => row.mbb_per_100 - row.ci95));
  const high = Math.max(0, ...rows.map((row) => row.mbb_per_100 + row.ci95));
  const span = high - low || 1;
  const at = (value) => `${(((value - low) / span) * 100).toFixed(2)}%`;
  // Three significant figures at every scale, as the README's headline writes them.
  const three = (value) => Number(value.toPrecision(3)).toLocaleString("en-US");

  board.replaceChildren(
    ...rows.map((row) => {
      const from = row.mbb_per_100 - row.ci95;
      const to = row.mbb_per_100 + row.ci95;
      const kind = from > 0 ? "win" : to < 0 ? "loss" : "open";
      const line = document.createElement("div");
      line.className = `row ${kind}`;
      const name = document.createElement("span");
      name.className = "name";
      name.textContent = row.opponent;
      const track = document.createElement("div");
      track.className = "track";
      track.setAttribute("role", "img");
      track.setAttribute(
        "aria-label",
        `${row.opponent}: ${three(row.mbb_per_100)} mbb/100, 95% interval ${three(from)} to ${three(to)}`,
      );
      const zero = document.createElement("span");
      zero.className = "zero";
      zero.style.left = at(0);
      const ci = document.createElement("span");
      ci.className = "ci";
      ci.style.left = at(from);
      ci.style.width = `calc(${at(to)} - ${at(from)})`;
      const dot = document.createElement("span");
      dot.className = "dot";
      dot.style.left = at(row.mbb_per_100);
      track.append(zero, ci, dot);
      const number = document.createElement("span");
      number.className = "num";
      const sign = row.mbb_per_100 >= 0 ? "+" : "";
      number.append(`${sign}${three(row.mbb_per_100)}`);
      const detail = document.createElement("small");
      detail.textContent = `±${three(row.ci95)} · ${row.deals} deals`;
      number.append(detail);
      line.append(name, track, number);
      return line;
    }),
  );
  provenance.textContent = `Played at commit ${run.commit}, champion ${run.champion}, seed ${run.seed}. Every line plays again bit for bit from these three.`;
})();
