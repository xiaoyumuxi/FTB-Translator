import { invoke } from "@tauri-apps/api/core";
import type { CmpEntry, CmpValidationReport } from "../models/cmp";
import type { LogLevel, SettingsData } from "../models/settings";
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
  return typedCall<{ entries: CmpEntry[] }>("load_cmp", { cmp_path: cmpPath });
}

export function saveCmpTargets(cmpPath: string, entries: CmpEntry[]) {
  return typedCall<{ saved: boolean; entries: number }>("save_cmp_targets", {
    cmp_path: cmpPath,
    edits: entries.map(({ index, target }) => ({ index, target })),
  });
}

export function validateCmp(
  request: { cmp_path: string; quests_dir: string },
  entries: CmpEntry[] = [],
) {
  return typedCall<CmpValidationReport>("validate_cmp", {
    ...request,
    edits: entries.map(({ index, target }) => ({ index, target })),
  });
}

export function applyCmp(request: { cmp_path: string; quests_dir: string }) {
  return typedCall<{ report: Report; run_id: number; task_id: string }>("apply_cmp", request);
}
