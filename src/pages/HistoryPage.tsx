import { useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { Archive, Check, CircleAlert, FileSearch, History, Trash2 } from "lucide-react";
import type { Run } from "../models/translation";
import { call, errorText, frontendLog } from "../services/tauri";

export function HistoryPage({
  runs,
  notify,
  reload,
}: {
  runs: Run[];
  notify: (value: string) => void;
  reload: () => void;
}) {
  const [query, setQuery] = useState("");
  const filtered = useMemo(
    () => runs.filter((run) => `${run.pack_name} ${run.quests_dir}`.toLowerCase().includes(query.toLowerCase())),
    [runs, query],
  );

  async function remove(id: number) {
    if (!window.confirm("删除这条历史记录？已经写入整合包的文件不会被删除。")) {
      void frontendLog("debug", "history_delete_cancelled", "用户取消删除历史记录", { run_id: id });
      return;
    }
    try {
      await call("history-delete", { run_id: id });
      void frontendLog("info", "history_deleted", "用户删除了历史记录", { run_id: id });
      reload();
      notify("历史记录已删除");
    } catch (error) {
      notify(errorText(error));
    }
  }

  async function exportRun(run: Run) {
    const target = await save({
      title: "导出汉化内容",
      defaultPath: `${run.pack_name || "ftb-translation"}-${run.id}.zip`,
      filters: [{ name: "ZIP 压缩包", extensions: ["zip"] }],
    });
    if (target) {
      try {
        await call("history-export", { run_id: run.id, path: target });
        void frontendLog("info", "history_exported", "用户导出了翻译历史", {
          run_id: run.id,
          path: target,
        });
        notify("ZIP 已导出");
      } catch (error) {
        notify(errorText(error));
      }
    }
  }

  return (
    <div className="page">
      <header className="page-header history-header">
        <div>
          <p className="eyebrow">TRANSLATION ARCHIVE</p>
          <h1>翻译历史</h1>
          <p>重新找到每一次写入、备份和可导出的汉化结果。</p>
        </div>
        <div className="search-box">
          <FileSearch />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索整合包或路径" />
        </div>
      </header>
      {filtered.length ? (
        <div className="history-list">
          {filtered.map((run) => (
            <article className="history-row" key={run.id}>
              <div className="history-date">
                <strong>
                  {new Date(run.created_at).toLocaleDateString("zh-CN", { month: "short", day: "numeric" })}
                </strong>
                <span>
                  {new Date(run.created_at).toLocaleTimeString("zh-CN", {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
              </div>
              <div className="history-main">
                <div>
                  <h2>{run.pack_name || "未命名整合包"}</h2>
                  <span className="mode-badge">{run.mode === "lang" ? "语言文件" : "章节文件"}</span>
                </div>
                <p>{run.quests_dir}</p>
                <div className="history-facts">
                  <span>
                    <Check />
                    {run.translated_entries} 条完成
                  </span>
                  <span className={run.warning_count ? "warning" : ""}>
                    <CircleAlert />
                    {run.warning_count} 条检查
                  </span>
                  <span>{run.model}</span>
                </div>
              </div>
              <div className="history-actions">
                <button className="secondary" onClick={() => exportRun(run)}>
                  <Archive />导出
                </button>
                <button className="icon-button danger" onClick={() => remove(run.id)} aria-label="删除">
                  <Trash2 />
                </button>
              </div>
            </article>
          ))}
        </div>
      ) : (
        <div className="empty-state">
          <div>
            <History />
          </div>
          <h2>{query ? "没有匹配的记录" : "还没有翻译历史"}</h2>
          <p>{query ? "换一个整合包名称或目录关键词。" : "完成第一次汉化后，结果会自动出现在这里。"}</p>
        </div>
      )}
    </div>
  );
}
