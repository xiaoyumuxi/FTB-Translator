import { useEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { CircleAlert, Download, FileCheck2, FileText, ShieldCheck, Upload } from "lucide-react";
import type { CmpDraft, CmpEntry, CmpValidationReport } from "../models/cmp";

export function CmpTable({
  draft: _draft,
  entries,
  setEntries,
  validation,
  validating,
  onOpen,
  onExport,
  onChoose,
  onValidate,
  onApply,
}: {
  draft: CmpDraft;
  entries: CmpEntry[];
  setEntries: Dispatch<SetStateAction<CmpEntry[]>>;
  validation: CmpValidationReport | null;
  validating: boolean;
  onOpen: () => void;
  onExport: () => void;
  onChoose: () => void;
  onValidate: () => void;
  onApply: () => void;
}) {
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("all");
  const [page, setPage] = useState(0);
  const pageSize = 60;
  const filtered = useMemo(
    () =>
      entries.filter(
        (entry) =>
          (status === "all" || entry.status === status) &&
          `${entry.file} ${entry.entry_id} ${entry.source} ${entry.target}`
            .toLowerCase()
            .includes(query.toLowerCase()),
      ),
    [entries, query, status],
  );
  const rows = filtered.slice(page * pageSize, page * pageSize + pageSize);
  const pages = Math.max(1, Math.ceil(filtered.length / pageSize));
  useEffect(() => setPage(0), [query, status]);

  function update(index: number, target: string) {
    setEntries((items) => items.map((item) => (item.index === index ? { ...item, target } : item)));
  }

  function label(value: string) {
    return value === "translated"
      ? "已翻译"
      : value === "rate_limited"
        ? "接口限流（可重试）"
        : value === "request_failed"
          ? "接口失败"
          : value === "format_guard"
            ? "格式需检查"
            : value === "review"
              ? "需检查"
              : "原文保留";
  }

  return (
    <section className="review-table card">
      <header className="review-table-header">
        <div>
          <p className="eyebrow">TRANSLATION REVIEW</p>
          <h2>在表格中校对，然后一次覆盖</h2>
          <p>直接修改中文列；点击确认后会校验格式、创建备份，并统一写入任务书。</p>
        </div>
        <div className="review-table-summary">
          <strong>{entries.length}</strong>
          <span>条译文</span>
        </div>
      </header>
      <div className="review-table-toolbar">
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="搜索英文、中文、文件或条目…"
        />
        <label className="status-filter">
          <span>状态</span>
          <select value={status} onChange={(event) => setStatus(event.target.value)}>
            <option value="all">全部状态</option>
            <option value="translated">已翻译</option>
            <option value="rate_limited">接口限流（可重试）</option>
            <option value="request_failed">接口失败</option>
            <option value="format_guard">格式需检查</option>
            <option value="review">需检查</option>
            <option value="unchanged">原文保留</option>
          </select>
        </label>
        <span>{filtered.length} 条匹配</span>
        <button className="secondary" onClick={onOpen}>
          <FileText />外部编辑 CMP
        </button>
        <button className="secondary" onClick={onExport}>
          <Download />导出
        </button>
        <button className="secondary" onClick={onChoose}>
          <Upload />导入 CMP
        </button>
        <button className="secondary" disabled={validating} onClick={onValidate}>
          <FileCheck2 />{validating ? "正在验证" : "验证 CMP"}
        </button>
        <button className="primary" onClick={onApply}>
          <ShieldCheck />确认并覆盖
        </button>
      </div>
      {validation && (
        <div className={`cmp-validation-summary ${validation.blocking ? "blocking" : "ready"}`}>
          <div>
            {validation.blocking ? <CircleAlert /> : <FileCheck2 />}
            <strong>{validation.blocking ? "发现阻断问题" : "CMP 可以安全应用"}</strong>
            <span>本次仅验证，未修改任何文件</span>
          </div>
          <dl>
            <div><dt>任务书归属</dt><dd>{validation.belongs_to_current_task_book ? "一致" : "不一致"}</dd></div>
            <div><dt>源指纹</dt><dd>{validation.source_fingerprint_matches ? "一致" : "不一致"}</dd></div>
            <div><dt>可应用</dt><dd>{validation.applicable_entries}</dd></div>
            <div><dt>格式失败</dt><dd>{validation.format_guard_failures}</dd></div>
            <div><dt>保持英文</dt><dd>{validation.unchanged_entries}</dd></div>
          </dl>
          {validation.files_to_modify.length > 0 && (
            <p>预计修改：{validation.files_to_modify.join("、")}</p>
          )}
          {validation.blocking_issues.length > 0 && (
            <ul>
              {validation.blocking_issues.map((issue, index) => <li key={`${index}-${issue}`}>{issue}</li>)}
            </ul>
          )}
        </div>
      )}
      <div className="review-table-scroll">
        <table>
          <thead>
            <tr>
              <th>来源</th>
              <th>英文原文</th>
              <th>中文译文（可编辑）</th>
              <th>状态</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((entry) => (
              <tr key={entry.index}>
                <td>
                  <code title={`${entry.file}\n${entry.entry_id}`}>{entry.entry_id}</code>
                  <small>{entry.file}</small>
                </td>
                <td>
                  <p>{entry.source}</p>
                </td>
                <td>
                  <textarea
                    aria-label={`第 ${entry.index + 1} 条中文译文`}
                    value={entry.target}
                    onChange={(event) => update(entry.index, event.target.value)}
                    rows={Math.min(5, Math.max(2, entry.target.split("\n").length))}
                  />
                </td>
                <td>
                  <span className={entry.status === "translated" ? "status-ok" : "status-review"}>
                    {label(entry.status)}
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <footer className="review-table-footer">
        <span>
          第 {Math.min(page * pageSize + 1, filtered.length)}–
          {Math.min((page + 1) * pageSize, filtered.length)} 条，共 {filtered.length} 条
        </span>
        <div>
          <button className="secondary" disabled={page === 0} onClick={() => setPage((value) => value - 1)}>
            上一页
          </button>
          <span>
            {page + 1} / {pages}
          </span>
          <button
            className="secondary"
            disabled={page >= pages - 1}
            onClick={() => setPage((value) => value + 1)}
          >
            下一页
          </button>
        </div>
      </footer>
    </section>
  );
}
