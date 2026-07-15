import type { ScanResult } from "../models/translation";
import { QuestMark } from "./QuestMark";

export function ScanSummary({ scan }: { scan: ScanResult }) {
  return (
    <div className="scan-summary">
      <div className="pack-row">
        <div className="pack-icon">
          <QuestMark compact />
        </div>
        <div>
          <span>已识别整合包</span>
          <strong>{scan.pack_name || "FTB Quests"}</strong>
        </div>
        <span className="mode-badge">{scan.mode_label}</span>
      </div>
      <div className="scan-stats">
        <div>
          <strong>{scan.entry_count.toLocaleString()}</strong>
          <span>可翻译条目</span>
        </div>
        <div>
          <strong>{scan.file_count}</strong>
          <span>{scan.mode === "lang" ? "语言文件" : "章节文件"}</span>
        </div>
        <div>
          <strong>{scan.estimated_batches}</strong>
          <span>预计请求批次</span>
        </div>
      </div>
      <div className="scan-files">
        {scan.files.map((file) => (
          <div key={file.path}>
            <code>{file.path}</code>
            <span>{file.entry_count} 条</span>
          </div>
        ))}
      </div>
      <p className="mono-path">{scan.source}</p>
    </div>
  );
}
