// The PGO workload's one decision: the fixed-seed commands it runs. The same list is what the
// workflow trains on and what the benchmark replays, so a profile cannot be trained on one shape
// of work and measured on another.
import { expect, test } from "bun:test";
import { workload } from "./pgo-workload";

test("the workload is fixed, seeded and covers both leagues", () => {
  expect(workload).toEqual([
    ["paired", "2000", "7"],
    ["arena", "2", "7", "20"],
    ["arena", "6-max", "2", "7", "20"],
  ]);
});
