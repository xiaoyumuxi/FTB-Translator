import type { CmpEntry } from "../models/cmp";

export type CmpQaFlag =
  | "review_status"
  | "unchanged"
  | "likely_untranslated"
  | "inconsistent";

export type CmpQaCounts = Record<CmpQaFlag, number> & {
  flagged_entries: number;
};

export type CmpQaInconsistentGroup = {
  source: string;
  targets: string[];
  entry_indexes: number[];
};

export type CmpQaReport = {
  flags_by_index: Map<number, Set<CmpQaFlag>>;
  counts: CmpQaCounts;
  inconsistent_groups: CmpQaInconsistentGroup[];
};

const REVIEW_STATUSES = new Set([
  "rate_limited",
  "request_failed",
  "format_guard",
  "review",
  "unchanged",
  "fallback",
]);
const LATIN_TEXT = /[A-Za-z]/;
const HAN_TEXT = /[\u3400-\u4dbf\u4e00-\u9fff]/u;

function normalized(value: string) {
  return value.trim().replace(/\s+/g, " ").toLocaleLowerCase();
}
function addFlag(
  flagsByIndex: Map<number, Set<CmpQaFlag>>,
  index: number,
  flag: CmpQaFlag,
) {
  const flags = flagsByIndex.get(index) ?? new Set<CmpQaFlag>();
  flags.add(flag);
  flagsByIndex.set(index, flags);
}

/**
 * Produces review hints only. These heuristics never decide whether a CMP may
 * be written; the Rust format guard remains the authoritative safety check.
 */
export function analyzeCmpEntries(entries: CmpEntry[]): CmpQaReport {
  const flagsByIndex = new Map<number, Set<CmpQaFlag>>();
  const bySource = new Map<string, CmpEntry[]>();

  for (const entry of entries) {
    const source = normalized(entry.source);
    const target = normalized(entry.target);
    if (REVIEW_STATUSES.has(entry.status)) {
      addFlag(flagsByIndex, entry.index, "review_status");
    }
    if (source === target) {
      addFlag(flagsByIndex, entry.index, "unchanged");
    } else if (LATIN_TEXT.test(entry.source) && !HAN_TEXT.test(entry.target)) {
      addFlag(flagsByIndex, entry.index, "likely_untranslated");
    }
    if (source) {
      const group = bySource.get(source) ?? [];
      group.push(entry);
      bySource.set(source, group);
    }
  }

  const inconsistentGroups: CmpQaInconsistentGroup[] = [];
  for (const group of bySource.values()) {
    if (group.length < 2) continue;
    const targets = new Map<string, string>();
    for (const entry of group) {
      const key = normalized(entry.target);
      if (key) targets.set(key, entry.target.trim());
    }
    if (targets.size < 2) continue;
    const entryIndexes = group.map((entry) => entry.index);
    for (const index of entryIndexes) addFlag(flagsByIndex, index, "inconsistent");
    inconsistentGroups.push({
      source: group[0].source,
      targets: [...targets.values()],
      entry_indexes: entryIndexes,
    });
  }

  const count = (flag: CmpQaFlag) =>
    [...flagsByIndex.values()].filter((flags) => flags.has(flag)).length;
  return {
    flags_by_index: flagsByIndex,
    counts: {
      review_status: count("review_status"),
      unchanged: count("unchanged"),
      likely_untranslated: count("likely_untranslated"),
      inconsistent: count("inconsistent"),
      flagged_entries: [...flagsByIndex.values()].filter((flags) => flags.size > 0).length,
    },
    inconsistent_groups: inconsistentGroups,
  };
}
