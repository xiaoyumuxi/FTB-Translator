export function ProgressPanel({ progress, done, total }: { progress: number; done: number; total: number }) {
  return (
    <>
      <h2>{progress < 100 ? "正在翻译" : "正在校验并写回"}</h2>
      <p>处理完成前请保留窗口，程序不会写入未经确认的译文。</p>
      <div className="progress-number">
        <strong>{progress}</strong>
        <span>%</span>
      </div>
      <div className="progress-detail">
        {total ? `已处理 ${done} / ${total} 个翻译单元` : "正在准备翻译单元…"}
      </div>
      <div className="progress-track">
        <span style={{ width: `${progress}%` }} />
      </div>
    </>
  );
}
