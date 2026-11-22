// The README's headline, tested at its one seam: the run the fleet's `headline.json` pins in, the
// block of text and its problems out. The run here is the shape `arena.json` holds.
import { expect, test } from "bun:test";
import { between, headline, pinnable, problems, replace } from "./headline";

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

test("the league's interval is wide enough for rows that share their deals", () => {
  // `verdict()` in learn/src/paired.rs sets ci = 1.96 · SE(mean), so (ci/1.96)² is the variance
  // of that row's mean. But the Arena plays the SAME deals against every opponent (arena.rs), so
  // the rows are not independent and their variances do not simply add. With the cards shared,
  // the honest bound is the perfectly-correlated one — the weighted mean of the per-row SEs, no
  // /sqrt(k) credit for more rows — which is this test's formula. The independent sum is the
  // narrower figure the code used to print, and it overclaims confidence the run does not have.
  const said = headline(clear());
  const correlated = 1.96 * (9 / 1.96 / 2 + 6 / 1.96 / 2);
  expect(said.aggregate.ci95).toBeCloseTo(correlated, 6);
  const independent = 1.96 * Math.sqrt(((9 / 1.96) ** 2 + (6 / 1.96) ** 2) / 4);
  expect(said.aggregate.ci95).toBeGreaterThan(independent);
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
  expect(block).toContain("over 5,000 deals");
  expect(block).toContain("95% CI ±7.50");
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
  expect(block).toContain("±20,000"); // the pooled half-width, at three figures
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

test("a run clearly below zero says behind, not a number that looks like a win", () => {
  // "Not distinguishable from zero" is a claim about the interval, and an interval below zero is
  // distinguishable from it. The block says the number and that it is behind, sign and all.
  const losing = clear({
    rows: [
      row({ opponent: "pricer", deals: 5_000, mbb_per_100: -42, ci95: 9 }),
      row({ opponent: "nit", deals: 5_000, mbb_per_100: -61, ci95: 6 }),
    ],
  });
  const block = headline(losing).block;
  expect(block).toContain("**-51.5 mbb/100 — behind**");
  expect(block).toContain("behind");
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

test("the league's deals are counted honestly: one set of deals against each opponent", () => {
  // The Arena plays the same deals against every opponent (`arena.rs`), so 6 rows of 100 are 100
  // deals played six ways, not 600 independent ones. The block says the smaller, true number.
  const block = headline(clear()).block; // two rows of 5,000 shares each
  expect(block).toContain("over 5,000 deals of the Arena across 2 opponents");
});

test("a pin whose shape is wrong is a problem line, never a thrown type error", () => {
  // `rows` that is not an array, a row that is null, a seed that is a string: each fails as the
  // malformed run it is, so the operator reads a reason rather than a stack trace.
  for (const broken of [
    { commit: "c", champion: "d", seed: 1, rows: "abc" },
    { commit: "c", champion: "d", seed: 1, rows: [null] },
    { commit: "c", champion: "d", seed: "x", rows: [] },
  ] as never[]) {
    expect(() => headline(broken)).not.toThrow();
    expect(headline(broken).problems.length).toBeGreaterThan(0);
  }
});

test("a malformed run is refused with a problem, not a crash", () => {
  const broken = { commit: "abc", champion: "d", seed: 1, rows: [{ opponent: "x" }] } as never;
  expect(headline(broken).problems).toEqual([
    "the pinned Arena run is malformed: the headline has nothing to say",
  ]);
  const negative = clear({ rows: [row({ deals: -5 }), row({ opponent: "nit", deals: 5 })] });
  expect(headline(negative).problems).toEqual([
    "the pinned Arena run is malformed: the headline has nothing to say",
  ]);
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

test("two pairs of markers are refused, so the check never rewrites the wrong block", () => {
  const block = headline(clear()).block;
  const twice = `${block}\n${block}\n`;
  expect(between(twice)).toEqual({
    problems: ["README.md's headline markers are not exactly one pair"],
  });
  // Content on the close marker's own line would be swallowed by a rewrite, so it is refused too.
  const trailing = "<!-- headline -->\nnumber\n<!-- /headline --> trailing\n";
  expect(between(trailing)).toEqual({
    problems: ["README.md's headline markers are not exactly one pair"],
  });
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

test("a big number is still said to three significant figures, not to the unit", () => {
  // An interval of ±201,399 over 100 deals cannot support the tens. The block says 201,000.
  expect(
    headline(clear({ rows: [row({ deals: 100, mbb_per_100: 200448.4, ci95: 201399.2 })] })).block,
  ).toContain("| pricer | +200,000 | ±201,000 | 100 |");
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

test("the pin takes the newest heads-up run, never a 6-max one", () => {
  // The headline is the heads-up league (V11-20); 6-max runs share arena.json beside them
  // (V11-29), and the newest of the file may be one.
  const six = { ...clear({ commit: "6666666" }), league: "6-max" };
  const heads = { ...clear({ commit: "2222222" }), league: "heads-up" };
  const old = clear({ commit: "0000000" });
  expect(pinnable([six, heads, old])?.commit).toBe("2222222");
  // A run stored before 6-max has no league, and was heads up.
  expect(pinnable([six, old])?.commit).toBe("0000000");
  expect(pinnable([six])).toBe(undefined);
  expect(pinnable(undefined)).toBe(undefined);
});
