import { useEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { CircleAlert, Download, FileCheck2, FileText, ShieldCheck, Upload } from "lucide-react";
import {
  analyzeCmpEntries,
  type CmpQaFlag,
} from "../lib/cmpQa";
import type { CmpDraft, CmpEntry, CmpValidationReport } from "../models/cmp";

type QualityFilter = "all" | "flagged" | CmpQaFlag;

export function CmpTable({
  draft,
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
  const [quality, setQuality] = useState<QualityFilter>("all");
  const [page, setPage] = useState(0);
  const pageSize = 60;
  const qa = useMemo(() => analyzeCmpEntries(entries), [entries]);
  const filtered = useMemo(
    () =>
      entries.filter((entry) => {
        const flags = qa.flags_by_index.get(entry.index);
        const qualityMatches =
          quality === "all" ||
          (quality === "flagged" ? Boolean(flags?.size) : Boolean(flags?.has(quality)));
        return (
          (status === "all" || entry.status === status) &&
          qualityMatches &&
          `${entry.file} ${entry.entry_id} ${entry.source} ${entry.target}`
            .toLowerCase()
            .includes(query.toLowerCase())
        );
      }),
    [entries, qa, query, quality, status],
  );
  const rows = filtered.slice(page * pageSize, page * pageSize + pageSize);
  const pages = Math.max(1, Math.ceil(filtered.length / pageSize));
  useEffect(() => setPage(0), [query, quality, status]);
  useEffect(() => setPage(0), [draft.cmp_path]);
  useEffect(() => {
    if (page >= pages) setPage(pages - 1);
  }, [page, pages]);

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

  function qaLabel(flag: CmpQaFlag) {
    return flag === "review_status"
      ? "状态需确认"
      : flag === "unchanged"
        ? "与原文相同"
        : flag === "likely_untranslated"
          ? "疑似未汉化"
          : "同源多译";
  }

  const qaMarks: { flag: CmpQaFlag; label: string; hint: string }[] = [
    { flag: "review_status", label: "状态需确认", hint: "接口失败、格式保护或人工审校状态" },
    { flag: "inconsistent", label: "同源多译", hint: "相同英文出现了不同中文" },
    { flag: "unchanged", label: "保持英文", hint: "可能是专名，也可能尚未翻译" },
    { flag: "likely_untranslated", label: "疑似未汉化", hint: "译文仍为拉丁文字且没有中文" },
  ];

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
      <section className="cmp-qa-ledger" aria-label="CMP 审校线索">
        <div className="cmp-qa-heading">
          <CircleAlert />
          <div>
            <strong>审校线索</strong>
            <span>启发式提示，不影响后端格式校验与写回权限</span>
          </div>
        </div>
        <div className="cmp-qa-marks">
          {qaMarks.map((mark) => (
            <button
              key={mark.flag}
              className={quality === mark.flag ? "active" : ""}
              title={mark.hint}
              onClick={() => setQuality((value) => (value === mark.flag ? "all" : mark.flag))}
            >
              <strong>{qa.counts[mark.flag]}</strong>
              <span>{mark.label}</span>
            </button>
          ))}
        </div>
        {qa.inconsistent_groups.length > 0 && (
          <div className="cmp-qa-examples">
            {qa.inconsistent_groups.slice(0, 3).map((group) => (
              <button
                key={`${group.source}-${group.entry_indexes.join("-")}`}
                title={`${group.source}\n${group.targets.join(" / ")}`}
                onClick={() => {
                  setQuality("inconsistent");
                  setQuery(group.source);
                }}
              >
                <span>{group.source}</span>
                <small>{group.targets.join(" / ")}</small>
              </button>
            ))}
          </div>
        )}
      </section>
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
        <label className="status-filter">
          <span>线索</span>
          <select
            value={quality}
            onChange={(event) => setQuality(event.target.value as QualityFilter)}
          >
            <option value="all">全部线索</option>
            <option value="flagged">仅看有线索</option>
            <option value="review_status">状态需确认</option>
            <option value="inconsistent">同源多译</option>
            <option value="unchanged">保持英文</option>
            <option value="likely_untranslated">疑似未汉化</option>
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
        <button className="primary" disabled={draft.can_apply === false} onClick={onApply}>
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
                  <div className="cmp-qa-tags">
                    {[...(qa.flags_by_index.get(entry.index) ?? [])]
                      .filter((flag) => flag !== "review_status")
                      .map((flag) => <small key={flag}>{qaLabel(flag)}</small>)}
                  </div>
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
