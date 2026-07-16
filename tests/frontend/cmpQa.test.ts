import assert from "node:assert/strict";
import test from "node:test";
import { analyzeCmpEntries } from "../../src/lib/cmpQa.ts";
import type { CmpEntry } from "../../src/models/cmp.ts";

function entry(
  index: number,
  source: string,
  target: string,
  status = "translated",
): CmpEntry {
  return {
    index,
    entry_id: `entry-${index}`,
    path: "$",
    file: "lang/en_us.snbt",
    source,
    target,
    status,
  };
}

test("groups inconsistent translations and preserves entry indexes", () => {
  const report = analyzeCmpEntries([
    entry(10, "Power", "能量"),
    entry(30, "  power  ", "动力"),
    entry(40, "Welcome", "Welcome", "unchanged"),
    entry(50, "Click here", "Open menu"),
    entry(60, "Machine", "机器"),
  ]);

  assert.deepEqual(report.counts, {
    review_status: 1,
    unchanged: 1,
    likely_untranslated: 1,
    inconsistent: 2,
    flagged_entries: 4,
  });
  assert.equal(report.inconsistent_groups.length, 1);
  assert.deepEqual(report.inconsistent_groups[0].entry_indexes, [10, 30]);
  assert.deepEqual(report.inconsistent_groups[0].targets, ["能量", "动力"]);
  assert.deepEqual([...report.flags_by_index.get(10)!], ["inconsistent"]);
  assert.equal(report.flags_by_index.has(60), false);
});
test("treats review hints as non-exclusive signals", () => {
  const report = analyzeCmpEntries([
    entry(1, "Mekanism", "Mekanism", "review"),
    entry(2, "Start", "开始", "format_guard"),
  ]);

  assert.deepEqual([...report.flags_by_index.get(1)!], ["review_status", "unchanged"]);
  assert.deepEqual([...report.flags_by_index.get(2)!], ["review_status"]);
  assert.equal(report.counts.flagged_entries, 2);
});
