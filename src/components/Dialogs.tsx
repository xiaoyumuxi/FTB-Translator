import { Check, FileText, Languages, Play, X } from "lucide-react";
import type { CmpDraft } from "../models/cmp";
import type { ScanResult } from "../models/translation";

export function CmpDecisionDialog({
  draft,
  onReview,
  onApply,
}: {
  draft: CmpDraft;
  onReview: () => void;
  onApply: () => void;
}) {
  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true">
        <div className="modal-icon">
          <FileText />
        </div>
        <p className="eyebrow">TRANSLATION READY</p>
        <h2>API 翻译完成，要直接覆盖吗？</h2>
        <p>
          已经生成包含英文 → 中文对照的 CMP
          文件。选择“否”可先人工修改；选择“是”会立即校验、创建备份并写回任务书。
        </p>
        <div className="confirm-target">
          <span>CMP 校对文件</span>
          <strong title={draft.cmp_path}>{draft.cmp_path.split(/[\\/]/).pop()}</strong>
        </div>
        <div className="modal-actions">
          <button className="secondary" onClick={onReview}>
            否，人工校对
          </button>
          <button className="primary" onClick={onApply}>
            <Check />是，直接覆盖
          </button>
        </div>
      </div>
    </div>
  );
}

export function ConfirmDialog({
  scan,
  onCancel,
  onConfirm,
}: {
  scan: ScanResult;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onCancel()}>
      <div className="modal" role="dialog" aria-modal="true">
        <button className="modal-close" onClick={onCancel}>
          <X />
        </button>
        <div className="modal-icon">
          <Languages />
        </div>
        <p className="eyebrow">READY TO TRANSLATE</p>
        <h2>翻译 {scan.entry_count.toLocaleString()} 条内容并生成 CMP？</h2>
        <p>
          本阶段只调用 API 并生成英文 → 中文校对文件，不会覆盖{" "}
          <code>{scan.mode === "lang" ? "lang" : "chapters"}</code>。确认应用 CMP 时才会创建备份并写回。
        </p>
        <div className="confirm-target">
          <span>最终写入目标</span>
          <strong>{scan.mode === "lang" ? "lang/zh_cn.snbt" : "chapters/*.snbt"}</strong>
        </div>
        <div className="modal-actions">
          <button className="secondary" onClick={onCancel}>
            暂不开始
          </button>
          <button className="primary" onClick={onConfirm}>
            <Play />翻译并生成 CMP
          </button>
        </div>
      </div>
    </div>
  );
}
