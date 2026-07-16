import type { ActiveTask } from "../models/task";

export type TaskRecoveryDecision =
  | { kind: "recover_translation"; activities: ActiveTask[] }
  | { kind: "writeback_blocked"; activities: ActiveTask[] }
  | { kind: "translation_active"; activities: ActiveTask[] }
  | { kind: "unknown"; activities: [] };

export function classifyTaskRecovery(activities: ActiveTask[]): TaskRecoveryDecision {
  const writeback = activities.filter((activity) => activity.state === "applying");
  if (writeback.length) return { kind: "writeback_blocked", activities: writeback };

  const recoverable = activities.filter(
    (activity) => activity.state === "translating" && activity.recoverable,
  );
  if (recoverable.length) return { kind: "recover_translation", activities: recoverable };

  const translating = activities.filter((activity) => activity.state === "translating");
  if (translating.length) return { kind: "translation_active", activities: translating };
  return { kind: "unknown", activities: [] };
}
