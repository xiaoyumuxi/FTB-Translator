import { invoke } from "@tauri-apps/api/core";
import type { LogLevel, SettingsData } from "../models/settings";

export function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
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

export function startTranslation(payload: SettingsData & { quests_dir: string; retry_cmp_path?: string }) {
  return invoke("start_translation", { payload });
}
