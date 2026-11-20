// The README's headline, tested at its one seam: the run the fleet's `headline.json` pins in, the
// block of text and its problems out. The run here is the shape `arena.json` holds.
import { expect, test } from "bun:test";
import { between, headline, problems, replace } from "./headline";

type Row = { opponent: string; deals: number; mbb_per_100: number; ci95: number };
type Run = { commit: string; champion: string; seed: number; rows: Row[] };

const row = (over: Partial<Row> = {}): Row => ({
  opponent: "pricer",
  deals: 100,
  mbb_per_100: 40210.5,
  ci95: 33125.25,
  ...over,
});

// A run whose pooled interval excludes zero: every row is far from zero on a tight interval.
const clear = (over: Partial<Run> = {}): Run => ({
  commit: "0dc0cb0",
  champion: "9f2c41a0",
  seed: 17,
  rows: [
    row({ opponent: "pricer", deals: 5_000, mbb_per_100: 42, ci95: 9 }),
    row({ opponent: "nit", deals: 5_000, mbb_per_100: 61, ci95: 6 }),
  ],
  ...over,
});

test("the headline weights every opponent by the deals behind it", () => {
  // Aggregate mbb/100 is the total won over the total hands, so a bout counts for each deal it
  // played rather than once: (40210.5 - 1200.5 + 8810) / 3 over equal deals.
  expect(headline(clear()).aggregate).toMatchObject({ deals: 10_000 });
  expect(headline(clear()).aggregate.mbb_per_100).toBeCloseTo((42 + 61) / 2, 10);
});

test("the league's interval pools the opponents' intervals by the deals behind them", () => {
  // A rate over a pooled sample has the variance of the pooled mean: each row's half-width is
  // 1.96 standard errors, so Var(row) = (ci/1.96)², and Var(pooled mean) = Σ (n_i/N)² Var(row)/n_i.
  const said = headline(clear());
  const pooled = ((9 / 1.96) ** 2 + (6 / 1.96) ** 2) / 5_000 / 4;
  expect(said.aggregate.ci95).toBeCloseTo(1.96 * Math.sqrt(pooled), 6);
});

test("the headline names the run it came from, and its seed", () => {
  const block = headline(clear()).block;
  expect(block).toContain("commit `0dc0cb0`");
  expect(block).toContain("champion `9f2c41a0`");
  expect(block).toContain("seed 17");
});

test("a run clear of zero is stated as a number, with its interval", () => {
  const block = headline(clear()).block;
  expect(block).toContain("mbb/100**");
  expect(block).toContain("over 10,000 deals");
  expect(block).toContain("95% CI ±0.0765");
});

test("a run that straddles zero is said to be a coin flip, never a number", () => {
  // A CI of ±274,386 on 50 deals covers zero: printing "+94,052 mbb/100" would be a claim the
  // sample cannot make. The block says what it knows, and how much it would take to know more.
  const noisy = clear({
    rows: [
      row({ opponent: "pricer", deals: 50, mbb_per_100: 5_000, ci95: 20_000 }),
      row({ opponent: "nit", deals: 50, mbb_per_100: -4_000, ci95: 20_000 }),
    ],
  });
  const block = headline(noisy).block;
  // mean +500, pooled half-width 2,000: the interval runs -1,500 to +2,500 and covers zero.
  expect(block).toContain("Not yet distinguishable from zero");
  expect(block).toContain("±2,000"); // the pooled half-width
  expect(block).not.toContain("mbb/100**");
});

test("a row with no interval makes the league's interval unmeasured, not exact", () => {
  // Zero variance is the widest claim a row can make, and it is the one row that measured
  // nothing: a pooled interval that took it as zero would be precision the run never had.
  const partial = clear({
    rows: [
      row({ opponent: "pricer", deals: 5_000, mbb_per_100: 42, ci95: 9 }),
      row({ opponent: "nit", deals: 5_000, mbb_per_100: 61, ci95: 0 }),
    ],
  });
  expect(headline(partial).aggregate.ci95).toBeUndefined();
  const block = headline(partial).block;
  expect(block).toContain("95% CI not measured");
  expect(block).toContain("Not yet distinguishable from zero");
  expect(block).not.toContain("mbb/100**");
});

test("a run clearly below zero is a number too, and a losing one", () => {
  // "Not distinguishable from zero" is a claim about the interval, and an interval below zero
  // is distinguishable from it. The block says the number it measured, sign and all.
  const losing = clear({
    rows: [
      row({ opponent: "pricer", deals: 5_000, mbb_per_100: -42, ci95: 9 }),
      row({ opponent: "nit", deals: 5_000, mbb_per_100: -61, ci95: 6 }),
    ],
  });
  const block = headline(losing).block;
  expect(block).toContain("**-51.5 mbb/100**");
  expect(block).not.toContain("Not yet distinguishable");
});

test("the headline keeps each opponent's line, with its interval", () => {
  const block = headline(clear()).block;
  expect(block).toContain("| pricer | +42.0 | ±9.00 | 5,000 |");
  expect(block).toContain("| nit | +61.0 | ±6.00 | 5,000 |");
});

test("the headline says what the league is not", () => {
  expect(headline(clear()).block).toContain("heads up");
  expect(headline(clear()).block).toContain("not the full benchmark");
});

test("a clone with no pin is refused, so a branch cannot delete the headline", () => {
  // The pin is a file of the repository. A branch that removed it would else pass with a README
  // saying nothing, which is the hand-typing this ticket exists to prevent.
  expect(headline(undefined).problems).toEqual([
    "no Arena run is pinned in fleet/headline.json: the headline has nothing to say",
  ]);
  expect(problems(undefined, "x\nanything\n")).toEqual([
    "no Arena run is pinned in fleet/headline.json: the headline has nothing to say",
  ]);
});

test("a run with no deals has no rate to report, and says why", () => {
  expect(headline(clear({ rows: [] })).problems).toEqual([
    "the pinned Arena run played no deals: the headline has nothing to say",
  ]);
});

test("a run whose deals add to none is refused rather than divided by zero", () => {
  expect(
    headline(clear({ rows: [row({ deals: 0, mbb_per_100: 500, ci95: 10 })] })).problems,
  ).toEqual(["the pinned Arena run's deals add to none: the headline has nothing to say"]);
});

test("the block carries its markers, and a missing one is a problem", () => {
  const block = headline(clear()).block;
  expect(block.startsWith("<!-- headline -->")).toBe(true);
  expect(block.trimEnd().endsWith("<!-- /headline -->")).toBe(true);
  expect(between(`Intro.\n\n${block}\nOutro.\n`)).toEqual({ text: block });
  expect(between("no markers here")).toEqual({ problems: ["README.md has no headline markers"] });
});

test("a README whose block differs from the pinned run is what the check refuses", () => {
  const stale = "<!-- headline -->\nold number\n<!-- /headline -->\n";
  expect(problems(clear(), `Intro.\n${stale}\n`)).toEqual([
    "README.md's headline is not the pinned run's",
  ]);
  expect(problems(clear(), `x\n${headline(clear()).block}y`)).toEqual([]);
});

test("writing the block back leaves the rest of the README as it was", () => {
  // The one write the tool does: a stale block in place, everything above and below untouched.
  expect(replace("Intro.\n<!-- headline -->\nold\n<!-- /headline -->\nOutro.\n", "NEW\n")).toEqual({
    text: "Intro.\nNEW\nOutro.\n",
  });
  expect(replace("no markers", "NEW\n")).toEqual({
    problems: ["README.md has no headline markers"],
  });
});

test("a number is said to three significant figures, and a missing interval is said as such", () => {
  // The interval is a 95% CI on a mean: zero means it was not measured, not that it was exact.
  const noInterval = clear({ rows: [row({ deals: 5_000, mbb_per_100: 7.4, ci95: 0 })] });
  expect(headline(noInterval).block).toContain("| pricer | +7.40 | not measured | 5,000 |");
});

test("the block is the same text on every write, so a fixed run is a fixed README", () => {
  // Nothing in the block may depend on the clock, a path or the order of a hash map.
  expect(headline(clear()).block).toBe(headline(clear()).block);
});
