import { useEffect, useRef } from "react";
import { ArrowRight } from "lucide-react";
import type { Activity } from "../models/translation";

export function TranslationLog({ entries }: { entries: Activity[] }) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.scrollTo({ top: box.current.scrollHeight, behavior: "smooth" });
  }, [entries.length]);

  return (
    <div className="card log-card activity-card">
      <div className="card-title compact">
        <div>
          <h2>实时翻译记录</h2>
          <p>自动跟随最新内容；英文与中文仅显示在本次界面，不写入诊断日志。</p>
        </div>
        <span className="live-dot">自动滚动</span>
      </div>
      <div className="log-list activity-list" ref={box}>
        {entries.map((entry, index) =>
          entry.type === "message" ? (
            <div className="activity-message" key={`m-${index}`}>
              <span>{String(index + 1).padStart(3, "0")}</span>
              <p>{entry.message}</p>
            </div>
          ) : (
            <article className="activity-translation" key={`t-${index}`}>
              <header>
                <code>{entry.entry_id}</code>
                <span className={entry.status === "translated" ? "status-ok" : "status-review"}>
                  {entry.status === "translated" ? "已翻译" : "需校对"}
                </span>
              </header>
              <div>
                <p>{entry.source}</p>
                <ArrowRight />
                <strong>{entry.target}</strong>
              </div>
            </article>
          ),
        )}
      </div>
    </div>
  );
}
