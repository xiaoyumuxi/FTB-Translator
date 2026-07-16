import assert from "node:assert/strict";
import test from "node:test";
import { classifyTaskRecovery } from "../../src/lib/taskRecovery.ts";
import type { ActiveTask } from "../../src/models/task.ts";

function activity(
  taskId: string,
  state: ActiveTask["state"],
  recoverable: boolean,
): ActiveTask {
  return {
    task_id: taskId,
    state,
    recoverable,
    updated_at: "2026-07-16T12:00:00Z",
  };
}

test("never offers automatic recovery for writeback", () => {
  const decision = classifyTaskRecovery([
    activity("old-translation", "translating", true),
    activity("writeback", "applying", false),
  ]);
  assert.equal(decision.kind, "writeback_blocked");
  assert.deepEqual(decision.activities.map((item) => item.task_id), ["writeback"]);
});
test("offers recovery only for translation records older than this process", () => {
  assert.equal(
    classifyTaskRecovery([activity("interrupted", "translating", true)]).kind,
    "recover_translation",
  );
  assert.equal(
    classifyTaskRecovery([activity("possibly-live", "translating", false)]).kind,
    "translation_active",
  );
  assert.equal(classifyTaskRecovery([]).kind, "unknown");
});
