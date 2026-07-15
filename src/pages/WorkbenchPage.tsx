import React, { type Dispatch, type SetStateAction } from "react";
import {
  Archive,
  ArrowRight,
  Check,
  ChevronRight,
  Copy,
  FileSearch,
  FileText,
  FolderOpen,
  Play,
  RefreshCw,
  Settings,
  ShieldCheck,
  Upload,
} from "lucide-react";
import type { CmpDraft, CmpEntry } from "../models/cmp";
import type { Activity, Report, ScanResult, Stage } from "../models/translation";
import { CmpTable } from "../components/CmpTable";
import { Metric } from "../components/Metric";
import { ProgressPanel } from "../components/ProgressPanel";
import { ScanSummary } from "../components/ScanSummary";
import { TranslationLog } from "../components/TranslationLog";

type WorkbenchPageProps = {
  stage: Stage;
  scan: ScanResult | null;
  path: string;
  setPath: (value: string) => void;
  busy: boolean;
  progress: number;
  progressDetail: { done: number; total: number };
  logs: Activity[];
  report: Report | null;
  warnings: number;
  cmpDraft: CmpDraft | null;
  cmpEntries: CmpEntry[];
  setCmpEntries: Dispatch<SetStateAction<CmpEntry[]>>;
  onChoose: () => void;
  onScan: () => void;
  onTranslate: () => void;
  onSettings: () => void;
  onOpenCmp: () => void;
  onExportCmp: () => void;
  onChooseCmp: () => void;
  onApplyCmp: () => void;
  onRetryRateLimited: () => void;
};

export function WorkbenchPage(props: WorkbenchPageProps) {
  const steps = [
    { key: "idle", label: "选择任务书" },
    { key: "running", label: "API 翻译" },
    { key: "review", label: "校对 CMP" },
    { key: "done", label: "完成写回" },
  ];
  const index = {
    idle: 0,
    scanned: 0,
    running: 1,
    review: 2,
    done: 3,
    error: props.scan ? 1 : 0,
  }[props.stage];
  const rateLimitedCount = props.cmpEntries.filter((entry) => entry.status === "rate_limited").length;

  return (
    <div className="page workbench-page">
      <header className="page-header">
        <div>
          <p className="eyebrow">TRANSLATION WORKBENCH</p>
          <h1>把任务书带给中文玩家</h1>
          <p>先生成可人工校对的 CMP 文件，确认后才会备份并写回。</p>
        </div>
        <div className={`status-pill ${props.stage}`}>
          <span />
          {props.stage === "running"
            ? "正在处理"
            : props.stage === "review"
              ? "等待校对"
              : props.stage === "done"
                ? "本次完成"
                : props.stage === "error"
                  ? "需要处理"
                  : props.stage === "scanned"
                    ? "等待开始"
                    : "准备就绪"}
        </div>
      </header>
      <section className="quest-chain" aria-label="汉化进度">
        {steps.map((step, stepIndex) => (
          <React.Fragment key={step.key}>
            <div
              className={`quest-step ${stepIndex <= index ? "active" : ""} ${stepIndex < index ? "complete" : ""}`}
            >
              <i>{stepIndex < index ? <Check /> : stepIndex + 1}</i>
              <span>{step.label}</span>
            </div>
            {stepIndex < 3 && (
              <div className={`quest-link ${stepIndex < index ? "active" : ""}`}>
                <span />
              </div>
            )}
          </React.Fragment>
        ))}
      </section>
      <div className="workspace-grid">
        <section className="card source-card">
          <div className="card-title">
            <div className="icon-tile blue">
              <FolderOpen />
            </div>
            <div>
              <h2>任务书位置</h2>
              <p>整合包根目录或 quests、lang、chapters 目录都可以</p>
            </div>
          </div>
          <div className="path-control">
            <input
              value={props.path}
              onChange={(event) => props.setPath(event.target.value)}
              placeholder="选择一个整合包目录…"
              onKeyDown={(event) => event.key === "Enter" && props.onScan()}
            />
            <button className="secondary" onClick={props.onChoose}>
              <FolderOpen />选择目录
            </button>
          </div>
          {props.scan ? (
            <ScanSummary scan={props.scan} />
          ) : (
            <div className="drop-hint" onClick={props.onChoose}>
              <FileSearch />
              <div>
                <strong>从扫描开始</strong>
                <span>我们会自动判断任务书格式，不会在扫描时改动文件。</span>
              </div>
              <ChevronRight />
            </div>
          )}
        </section>
        <aside className="card action-card">
          <p className="eyebrow">NEXT ACTION</p>
          {props.stage === "idle" || (props.stage === "error" && !props.scan) ? (
            <>
              <h2>先找到任务书</h2>
              <p>扫描只读取目录结构和可翻译条目，不会覆盖任何文件。</p>
              <button className="primary wide" disabled={props.busy} onClick={props.onScan}>
                {props.busy ? <RefreshCw className="spin" /> : <FileSearch />}
                扫描任务书
              </button>
            </>
          ) : props.stage === "scanned" ? (
            <>
              <h2>生成校对文件</h2>
              <p>API 翻译完成后先生成 CMP，确认之前不会修改任务书。</p>
              <button className="primary wide" onClick={props.onTranslate}>
                <Play />开始翻译<ArrowRight />
              </button>
              <button className="text-button" onClick={props.onChooseCmp}>
                <Upload />选择已有 CMP
              </button>
              <button className="text-button" onClick={props.onSettings}>
                <Settings />检查翻译设置
              </button>
            </>
          ) : props.stage === "running" ? (
            <ProgressPanel
              progress={props.progress}
              done={props.progressDetail.done}
              total={props.progressDetail.total}
            />
          ) : props.stage === "review" ? (
            <>
              <h2>等待人工确认</h2>
              <p>可以编辑 CMP 右侧中文，完成后再应用并覆盖任务书。</p>
              <div className="result-mini amber">
                <FileText />
                <span>CMP 尚未写回</span>
              </div>
              <button className="primary wide" onClick={props.onApplyCmp}>
                <ShieldCheck />校验并覆盖
              </button>
            </>
          ) : (
            <>
              <h2>汉化已经写入</h2>
              <p>
                {props.warnings
                  ? `有 ${props.warnings} 条内容建议人工确认，其余内容已完成。`
                  : "格式检查全部通过，可以进入游戏查看效果。"}
              </p>
              <div className="result-mini">
                <Check />
                <span>备份已创建</span>
              </div>
              <button className="secondary wide" onClick={props.onChoose}>
                翻译另一个整合包
              </button>
            </>
          )}
        </aside>
      </div>
      {props.stage === "review" && props.cmpDraft && (
        <>
          <div className="rate-limit-retry-bar">
            <div>
              <RefreshCw />
              <span>
                <strong>{rateLimitedCount} 条因接口限流未翻译</strong>
                <small>只重新请求这一批，其他译文和人工修改保持不变。</small>
              </span>
            </div>
            <button
              className="secondary"
              disabled={props.busy || rateLimitedCount === 0}
              onClick={props.onRetryRateLimited}
            >
              <RefreshCw />重试限流项
            </button>
          </div>
          <CmpTable
            draft={props.cmpDraft}
            entries={props.cmpEntries}
            setEntries={props.setCmpEntries}
            onOpen={props.onOpenCmp}
            onExport={props.onExportCmp}
            onChoose={props.onChooseCmp}
            onApply={props.onApplyCmp}
          />
        </>
      )}
      {(props.logs.length > 0 || props.report) && (
        <section className="lower-grid">
          <TranslationLog entries={props.logs} />
          {props.report && (
            <div className="card report-card">
              <div className="card-title compact">
                <div>
                  <h2>本次结果</h2>
                  <p>
                    {props.report.translated_entries} / {props.report.total_entries} 条已处理
                  </p>
                </div>
              </div>
              <Metric label="缓存命中" value={props.report.cache_hits} />
              <Metric label="需要检查" value={props.warnings} warn={props.warnings > 0} />
              <Metric
                label="翻译失败"
                value={props.report.failed_entries.length}
                warn={props.report.failed_entries.length > 0}
              />
              <div className="backup-path">
                <Archive />
                <span title={props.report.backup_dir}>{props.report.backup_dir}</span>
                <button
                  onClick={() => navigator.clipboard.writeText(props.report!.backup_dir)}
                  aria-label="复制备份路径"
                >
                  <Copy />
                </button>
              </div>
            </div>
          )}
        </section>
      )}
    </div>
  );
}
