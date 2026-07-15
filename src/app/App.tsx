import { useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { BookOpen, Check, History, Moon, Settings, Sun } from "lucide-react";
import { CmpDecisionDialog, ConfirmDialog } from "../components/Dialogs";
import { Nav } from "../components/Nav";
import { QuestMark } from "../components/QuestMark";
import { useTauriEvents } from "../hooks/useTauriEvents";
import type { CmpDraft, CmpEntry } from "../models/cmp";
import {
  defaultSettings,
  providerOptions,
  type Provider,
  type SettingsData,
} from "../models/settings";
import {
  note,
  type Activity,
  type Report,
  type Run,
  type ScanResult,
  type Stage,
  type View,
} from "../models/translation";
import { HistoryPage } from "../pages/HistoryPage";
import { SettingsPage } from "../pages/SettingsPage";
import { WorkbenchPage } from "../pages/WorkbenchPage";
import { call, errorText, frontendLog, startTranslation } from "../services/tauri";

export function App() {
  const [view, setView] = useState<View>("workbench");
  const [stage, setStage] = useState<Stage>("idle");
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    localStorage.theme === "dark" ? "dark" : "light",
  );
  const [settings, setSettings] = useState<SettingsData>(defaultSettings);
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [selectedPath, setSelectedPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [progressDetail, setProgressDetail] = useState<{ done: number; total: number }>({ done: 0, total: 0 });
  const progressTarget = useRef({ done: 0, total: 0 });
  const progressDisplayed = useRef({ done: 0, total: 0 });
  const progressTimer = useRef<number | null>(null);
  const retryingRateLimited = useRef(false);
  const [logs, setLogs] = useState<Activity[]>([]);
  const [report, setReport] = useState<Report | null>(null);
  const [runs, setRuns] = useState<Run[]>([]);
  const [toast, setToast] = useState("");
  const [confirm, setConfirm] = useState(false);
  const [cmpDraft, setCmpDraft] = useState<CmpDraft | null>(null);
  const [cmpEntries, setCmpEntries] = useState<CmpEntry[]>([]);
  const [reviewPrompt, setReviewPrompt] = useState(false);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.theme = theme;
    void frontendLog("debug", "theme_applied", "界面主题已应用", { theme });
  }, [theme]);

  useEffect(() => {
    void frontendLog("info", "frontend_started", "前端界面已启动", { language: navigator.language });
    const onError = (event: ErrorEvent) =>
      void frontendLog("error", "window_error", "前端发生未捕获错误", {
        error: event.message,
        source: event.filename,
        line: event.lineno,
        column: event.colno,
      });
    const onRejection = (event: PromiseRejectionEvent) =>
      void frontendLog("error", "unhandled_rejection", "前端 Promise 未处理异常", {
        error: errorText(event.reason),
      });
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
      stopProgressAnimation();
    };
  }, []);

  useEffect(() => {
    call<SettingsData>("settings")
      .then((value) => {
        setSettings({ ...value, api_key: "", api_key_changed: false });
        void frontendLog("info", "settings_loaded", "前端设置已加载", {
          provider: value.provider,
          log_level: value.log_level,
        });
      })
      .catch((error) => notify(String(error)));
  }, []);

  useTauriEvents((event) => {
    if (event.type === "log" && event.message) {
      setLogs((value) => [...value.slice(-299), note(event.message!)]);
    }
    if (
      event.type === "translation_preview" &&
      event.entry_id !== undefined &&
      event.source !== undefined &&
      event.target !== undefined
    ) {
      setLogs((value) => [
        ...value.slice(-299),
        {
          type: "translation",
          entry_id: event.entry_id!,
          source: event.source!,
          target: event.target!,
          status: event.status || "translated",
        },
      ]);
    }
    if (event.type === "progress") {
      queueProgress(event.done || 0, event.total || 0);
      void frontendLog("trace", "translation_progress", "前端收到翻译进度", {
        task_id: event.task_id || "",
        done: event.done || 0,
        total: event.total || 0,
        stage: event.stage || "",
      });
    }
    if (event.type === "review_ready" && event.cmp_path) {
      const draft = {
        cmp_path: event.cmp_path,
        task_id: event.task_id,
        total_entries: event.total_entries || 0,
        warning_count: event.warning_count || 0,
        failed_count: event.failed_count || 0,
      };
      retryingRateLimited.current = false;
      stopProgressAnimation();
      setBusy(false);
      setProgress(100);
      setStage("review");
      setCmpDraft(draft);
      void loadCmpEntries(draft);
      setReviewPrompt(true);
      setLogs((value) => [...value, note("API 翻译完成，已打开可编辑校对表格，尚未覆盖任务书。")]);
      void frontendLog("info", "cmp_review_ready", "CMP 校对文件已生成", draft);
      notify("翻译完成，请确认是否直接覆盖");
    }
    if (event.type === "done" && event.report) {
      retryingRateLimited.current = false;
      stopProgressAnimation();
      setBusy(false);
      setProgress(100);
      setStage("done");
      setReport(event.report);
      setLogs((value) => [...value, note("翻译完成，输出与备份均已写入。")]);
      void frontendLog("info", "translation_completed", "前端收到翻译完成事件", {
        run_id: event.run_id,
        total: event.report.total_entries,
        translated: event.report.translated_entries,
        failed: event.report.failed_entries.length,
        warnings: Object.keys(event.report.warnings).length,
      });
      notify("任务书汉化完成");
      loadHistory();
    }
    if (event.type === "error") {
      const retrying = retryingRateLimited.current;
      retryingRateLimited.current = false;
      stopProgressAnimation();
      setBusy(false);
      setStage(retrying ? "review" : "scanned");
      void frontendLog("error", "translation_failed", "前端收到翻译失败事件", {
        task_id: event.task_id || "",
        error: event.message || "翻译失败",
      });
      notify(event.message || "翻译失败");
    }
  });

  const notify = (text: string) => {
    setToast(text);
    window.setTimeout(() => setToast(""), 3200);
  };
  const loadHistory = () => call<Run[]>("history-list").then(setRuns).catch((error) => notify(String(error)));

  useEffect(() => {
    if (view === "history") loadHistory();
  }, [view]);

  function stopProgressAnimation() {
    if (progressTimer.current !== null) {
      window.clearInterval(progressTimer.current);
      progressTimer.current = null;
    }
  }

  function resetProgress(total = 0) {
    stopProgressAnimation();
    progressTarget.current = { done: 0, total };
    progressDisplayed.current = { done: 0, total };
    setProgress(0);
    setProgressDetail({ done: 0, total });
  }

  function queueProgress(done: number, total: number) {
    if (total <= 0) {
      resetProgress();
      return;
    }
    const safeDone = Math.min(done, total);
    if (progressDisplayed.current.total !== total) {
      stopProgressAnimation();
      progressDisplayed.current = { done: 0, total };
      setProgressDetail({ done: 0, total });
      setProgress(0);
    }
    progressTarget.current = { done: safeDone, total };
    if (progressTimer.current !== null) return;
    progressTimer.current = window.setInterval(() => {
      const target = progressTarget.current;
      const shown = progressDisplayed.current;
      if (shown.done >= target.done) {
        stopProgressAnimation();
        return;
      }
      const next = { done: shown.done + 1, total: target.total };
      progressDisplayed.current = next;
      setProgressDetail(next);
      setProgress(Math.min(100, Math.round((next.done / next.total) * 100)));
      if (next.done >= target.done) stopProgressAnimation();
    }, 24);
  }

  function navigate(next: View) {
    void frontendLog("debug", "navigation_changed", "用户切换页面", { from: view, to: next });
    setView(next);
  }

  async function chooseFolder() {
    const value = await open({ directory: true, multiple: false, title: "选择整合包或 FTB Quests 目录" });
    if (typeof value === "string") {
      void frontendLog("info", "folder_selected", "用户选择了任务书目录", { path: value });
      setSelectedPath(value);
      await doScan(value);
    } else {
      void frontendLog("debug", "folder_selection_cancelled", "用户取消了目录选择");
    }
  }

  async function doScan(path = selectedPath) {
    if (!path.trim()) {
      void frontendLog("warn", "scan_rejected", "扫描未开始：目录为空");
      return notify("请先选择整合包目录");
    }
    resetProgress();
    setBusy(true);
    setReport(null);
    setCmpDraft(null);
    setCmpEntries([]);
    setReviewPrompt(false);
    void frontendLog("info", "scan_started", "用户开始扫描任务书", { path });
    try {
      const result = await call<ScanResult>("scan", { path, batch_size: settings.batch_size });
      setScan(result);
      setSelectedPath(result.quests_dir);
      setStage("scanned");
      setLogs([note(`已找到 ${result.entry_count} 条可翻译文本。`), note(`源目录：${result.source}`)]);
      void frontendLog("info", "scan_completed", "前端已展示扫描结果", {
        mode: result.mode,
        entries: result.entry_count,
        files: result.file_count,
      });
    } catch (error) {
      setStage("error");
      notify(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function beginTranslation() {
    setConfirm(false);
    if (!scan) {
      void frontendLog("warn", "translation_rejected", "翻译未开始：没有扫描结果");
      return;
    }
    retryingRateLimited.current = false;
    resetProgress();
    setBusy(true);
    setStage("running");
    setLogs([note("正在启动安全翻译任务…")]);
    void frontendLog("info", "translation_started", "用户确认开始翻译", {
      quests_dir: scan.quests_dir,
      provider: settings.provider,
    });
    try {
      await startTranslation({ quests_dir: scan.quests_dir, ...settings });
    } catch (error) {
      void frontendLog("error", "translation_start_failed", "启动翻译命令失败", {
        error: errorText(error),
      });
      setBusy(false);
      setStage("scanned");
      notify(String(error));
    }
  }

  async function retryRateLimited() {
    if (!scan || !cmpDraft) return;
    const count = cmpEntries.filter((entry) => entry.status === "rate_limited").length;
    if (!count) return notify("当前没有可重试的限流条目");
    try {
      await call("cmp-save-edits", { cmp_path: cmpDraft.cmp_path, entries: cmpEntries });
      retryingRateLimited.current = true;
      resetProgress(count);
      setBusy(true);
      setStage("running");
      setLogs((value) => [...value, note(`正在重新调用翻译接口处理 ${count} 条限流内容…`)]);
      void frontendLog("info", "rate_limited_retry_started", "用户重试限流条目", {
        task_id: cmpDraft.task_id || "",
        cmp_path: cmpDraft.cmp_path,
        count,
      });
      await startTranslation({
        quests_dir: scan.quests_dir,
        retry_cmp_path: cmpDraft.cmp_path,
        ...settings,
      });
    } catch (error) {
      retryingRateLimited.current = false;
      setBusy(false);
      setStage("review");
      void frontendLog("error", "rate_limited_retry_failed", "限流条目重试启动失败", {
        task_id: cmpDraft.task_id || "",
        error: errorText(error),
      });
      notify(String(error));
    }
  }

  async function loadCmpEntries(draft: CmpDraft) {
    try {
      const result = await call<{ entries: CmpEntry[] }>("cmp-review", { cmp_path: draft.cmp_path });
      setCmpEntries(result.entries);
    } catch (error) {
      notify(String(error));
    }
  }

  async function applyCmp() {
    if (!scan || !cmpDraft) return;
    setReviewPrompt(false);
    setBusy(true);
    setStage("running");
    setLogs((value) => [...value, note("正在校验校对表格、创建备份并覆盖任务书…")]);
    void frontendLog("info", "cmp_apply_started", "用户确认校对表格并应用 CMP", {
      task_id: cmpDraft.task_id || "",
      cmp_path: cmpDraft.cmp_path,
      entries: cmpEntries.length,
    });
    try {
      if (cmpEntries.length) {
        await call("cmp-save-edits", { cmp_path: cmpDraft.cmp_path, entries: cmpEntries });
      }
      const result = await call<{ report: Report; run_id: number; task_id: string }>("cmp-apply", {
        cmp_path: cmpDraft.cmp_path,
        quests_dir: scan.quests_dir,
      });
      setBusy(false);
      setProgress(100);
      setStage("done");
      setReport(result.report);
      setLogs((value) => [...value, note("校对表格已通过校验，翻译结果已写入任务书。")]);
      void frontendLog("info", "cmp_applied", "CMP 已应用", {
        task_id: result.task_id,
        run_id: result.run_id,
        cmp_path: cmpDraft.cmp_path,
      });
      notify("任务书汉化完成");
      loadHistory();
    } catch (error) {
      void frontendLog("warn", "cmp_apply_failed", "CMP 校验或应用失败", {
        task_id: cmpDraft.task_id || "",
        cmp_path: cmpDraft.cmp_path,
        error: errorText(error),
      });
      setBusy(false);
      setStage("review");
      notify(String(error));
    }
  }

  async function openCmp() {
    if (!cmpDraft) return;
    try {
      await call("cmp-open", { cmp_path: cmpDraft.cmp_path });
      void frontendLog("info", "cmp_opened", "用户打开了 CMP 校对文件", {
        task_id: cmpDraft.task_id || "",
        cmp_path: cmpDraft.cmp_path,
      });
    } catch (error) {
      notify(String(error));
    }
  }

  async function exportCmp() {
    if (!cmpDraft) return;
    const target = await save({
      title: "导出 CMP 校对文件",
      defaultPath: "ftb-translation-review.cmp",
      filters: [{ name: "FTB CMP 校对文件", extensions: ["cmp"] }],
    });
    if (!target) {
      void frontendLog("debug", "cmp_export_cancelled", "用户取消另存 CMP", {
        task_id: cmpDraft.task_id || "",
      });
      return;
    }
    try {
      await call("cmp-export", { cmp_path: cmpDraft.cmp_path, path: target });
      void frontendLog("info", "cmp_exported", "用户另存了 CMP 校对文件", {
        task_id: cmpDraft.task_id || "",
        path: target,
      });
      notify("CMP 校对文件已导出");
    } catch (error) {
      notify(String(error));
    }
  }

  async function chooseCmp() {
    const value = await open({
      multiple: false,
      directory: false,
      title: "选择 CMP 校对文件",
      filters: [{ name: "FTB CMP 校对文件", extensions: ["cmp"] }],
    });
    if (typeof value !== "string") {
      void frontendLog("debug", "cmp_selection_cancelled", "用户取消选择 CMP 校对文件");
      return;
    }
    const draft = cmpDraft
      ? { ...cmpDraft, cmp_path: value, task_id: undefined }
      : {
          cmp_path: value,
          total_entries: scan?.entry_count || 0,
          warning_count: 0,
          failed_count: 0,
        };
    setCmpDraft(draft);
    setStage("review");
    setReviewPrompt(false);
    await loadCmpEntries(draft);
    void frontendLog("info", "cmp_selected", "用户选择了 CMP 校对文件", { cmp_path: value });
    notify("已打开 CMP 校对表格");
  }

  function reviewCmp() {
    if (cmpDraft) {
      void frontendLog("info", "cmp_manual_review_selected", "用户选择先人工校对 CMP", {
        task_id: cmpDraft.task_id || "",
        cmp_path: cmpDraft.cmp_path,
      });
    }
    setReviewPrompt(false);
  }

  async function saveSettings() {
    try {
      const result = await call<{ credential_backend: string; glossary_path: string }>("save-settings", settings);
      setSettings((value) => ({
        ...value,
        api_key: "",
        api_key_changed: false,
        has_api_key: value.api_key_changed ? !!value.api_key.trim() : value.has_api_key,
        credential_backend: result.credential_backend,
        glossary_path: result.glossary_path,
      }));
      void frontendLog("info", "settings_saved", "用户保存了设置", {
        provider: settings.provider,
        log_level: settings.log_level,
        glossary_enabled: settings.glossary_enabled,
      });
      notify("设置已保存");
    } catch (error) {
      notify(String(error));
    }
  }

  function changeProvider(provider: Provider) {
    void frontendLog("info", "provider_changed", "用户切换翻译提供商", {
      from: settings.provider,
      to: provider,
    });
    const preset = providerOptions[provider];
    setSettings((value) => ({
      ...value,
      provider,
      api_key: "",
      api_key_changed: false,
      has_api_key: false,
      base_url: preset.base_url,
      model: preset.model,
      glossary_enabled: preset.supportsGlossary ? value.glossary_enabled : false,
      batch_size: preset.supportsTaskParameters ? value.batch_size : "auto",
      concurrency: preset.supportsTaskParameters ? value.concurrency : "auto",
    }));
  }

  const warningCount = report ? Object.keys(report.warnings).length : 0;

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <QuestMark />
          <div>
            <strong>FTB Translater</strong>
            <span>任务书汉化工作台</span>
          </div>
        </div>
        <nav aria-label="主导航">
          <Nav
            active={view === "workbench"}
            icon={<BookOpen />}
            label="翻译工作台"
            onClick={() => navigate("workbench")}
          />
          <Nav
            active={view === "history"}
            icon={<History />}
            label="翻译历史"
            onClick={() => navigate("history")}
            badge={runs.length || undefined}
          />
          <Nav
            active={view === "settings"}
            icon={<Settings />}
            label="服务设置"
            onClick={() => navigate("settings")}
          />
        </nav>
        <button className="theme-toggle" onClick={() => setTheme(theme === "light" ? "dark" : "light")}>
          {theme === "light" ? <Moon /> : <Sun />}
          <span>{theme === "light" ? "切换深色" : "切换浅色"}</span>
        </button>
      </aside>
      <main className="main-area">
        {view === "workbench" && (
          <WorkbenchPage
            stage={stage}
            scan={scan}
            path={selectedPath}
            setPath={setSelectedPath}
            busy={busy}
            progress={progress}
            progressDetail={progressDetail}
            logs={logs}
            report={report}
            warnings={warningCount}
            cmpDraft={cmpDraft}
            cmpEntries={cmpEntries}
            setCmpEntries={setCmpEntries}
            onChoose={chooseFolder}
            onScan={() => doScan()}
            onTranslate={() => setConfirm(true)}
            onSettings={() => navigate("settings")}
            onOpenCmp={openCmp}
            onExportCmp={exportCmp}
            onChooseCmp={chooseCmp}
            onApplyCmp={applyCmp}
            onRetryRateLimited={retryRateLimited}
          />
        )}
        {view === "settings" && (
          <SettingsPage
            value={settings}
            onChange={setSettings}
            onProviderChange={changeProvider}
            onSave={saveSettings}
            notify={notify}
          />
        )}
        {view === "history" && <HistoryPage runs={runs} notify={notify} reload={loadHistory} />}
      </main>
      {confirm && scan && (
        <ConfirmDialog scan={scan} onCancel={() => setConfirm(false)} onConfirm={beginTranslation} />
      )}
      {reviewPrompt && cmpDraft && (
        <CmpDecisionDialog draft={cmpDraft} onReview={reviewCmp} onApply={applyCmp} />
      )}
      {toast && (
        <div className="toast">
          <Check />
          {toast}
        </div>
      )}
    </div>
  );
}
