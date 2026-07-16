import { invoke } from "@tauri-apps/api/core";
import type { CmpEntry, CmpValidationReport, TaskState } from "../models/cmp";
import type { LogLevel, SettingsData } from "../models/settings";
import type { ActiveTask } from "../models/task";
import type { Report, ScanResult } from "../models/translation";

export function errorText(error: unknown) {
  if (error instanceof Error) return error.message;
  if (
    typeof error === "object" &&
    error !== null &&
    "user_message" in error &&
    typeof error.user_message === "string"
  ) {
    return error.user_message;
  }
  return String(error);
}

export function errorCode(error: unknown) {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof error.code === "string"
  ) {
    return error.code;
  }
  return undefined;
}

export function frontendLog(
  level: LogLevel,
  event: string,
  message: string,
  context: Record<string, unknown> = {},
) {
  return invoke("bridge", {
    command: "frontend-log",
    payload: { level, event, message, context },
  }).catch((error) => console.error("frontend log write failed", error));
}

export async function call<T>(command: string, payload: Record<string, unknown> = {}) {
  try {
    return await invoke<T>("bridge", { command, payload });
  } catch (error) {
    void frontendLog("error", "bridge_call_failed", "前端调用后端命令失败", {
      command,
      error: errorText(error),
    });
    throw error;
  }
}

async function typedCall<T>(command: string, request: object) {
  try {
    return await invoke<T>(command, { request });
  } catch (error) {
    void frontendLog("error", "typed_command_failed", "前端调用强类型后端命令失败", {
      command,
      error: errorText(error),
    });
    throw error;
  }
}

export function scanTask(request: { path: string; batch_size: string }) {
  return typedCall<ScanResult>("scan", request);
}

export function translateTask(
  questsDir: string,
  settings: SettingsData,
  retryCmpPath?: string,
) {
  return typedCall<{ accepted: boolean; task_id: string }>("translate", {
    quests_dir: questsDir,
    retry_cmp_path: retryCmpPath,
    api_key: settings.api_key,
    provider: settings.provider,
    base_url: settings.base_url,
    model: settings.model,
    style: settings.style,
    batch_size: settings.batch_size,
    concurrency: settings.concurrency,
    glossary_enabled: settings.glossary_enabled,
    glossary_path: settings.glossary_path,
  });
}

export function loadCmp(cmpPath: string) {
  return typedCall<{
    entries: CmpEntry[];
    task_id: string;
    task_state: TaskState;
    can_apply: boolean;
    cmp_revision: string;
  }>("load_cmp", { cmp_path: cmpPath });
}

export function saveCmpTargets(cmpPath: string, expectedRevision: string, entries: CmpEntry[]) {
  return typedCall<{ saved: boolean; entries: number; cmp_revision: string }>("save_cmp_targets", {
    cmp_path: cmpPath,
    expected_revision: expectedRevision,
    edits: entries.map(({ index, target }) => ({ index, target })),
  });
}

export function validateCmp(
  request: { cmp_path: string; quests_dir: string; cmp_revision: string },
  entries: CmpEntry[] = [],
) {
  return typedCall<CmpValidationReport>("validate_cmp", {
    ...request,
    edits: entries.map(({ index, target }) => ({ index, target })),
  });
}

export function applyCmp(request: { cmp_path: string; quests_dir: string; cmp_revision: string }) {
  return typedCall<{
    report: Report;
    run_id: number;
    task_id: string;
    post_commit_warnings: string[];
  }>("apply_cmp", request);
}

export function recoverInterruptedTranslation(questsDir: string) {
  return typedCall<{ recovered: number }>("recover_translation", { quests_dir: questsDir });
}

export function inspectTaskState(questsDir: string) {
  return typedCall<{ activities: ActiveTask[] }>("inspect_task_state", {
    quests_dir: questsDir,
  });
}
