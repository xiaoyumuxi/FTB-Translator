from __future__ import annotations

import queue
import threading
from pathlib import Path
from tkinter import filedialog, messagebox

import customtkinter as ctk

from ftb_translater.config import load_api_key, save_api_key
from ftb_translater.deepseek_client import DEFAULT_MODEL
from ftb_translater.chapters import count_chapter_segments
from ftb_translater.logger import get_logger, setup_logging
from ftb_translater.paths import detect_source_mode, resolve_quests_dir, source_lang_path
from ftb_translater.snbt import load_lang_snbt
from ftb_translater.translator import AUTO_BATCH_MAX_ENTRIES, estimate_batches, translate_quests_auto

_log = get_logger(__name__)


class FtbTranslaterApp(ctk.CTk):
    def __init__(self):
        super().__init__()
        setup_logging()
        _log.info("FTB Translater starting up")
        self.title("FTB Translater")
        self.geometry("760x520")
        self.minsize(720, 480)

        self.selected_dir = ctk.StringVar()
        self.api_key = ctk.StringVar(value=load_api_key())
        self.status = ctk.StringVar(value="请选择整合包目录，或它下面的 config/ftbquests/quests/lang/chapters 任一目录。")
        self.summary = ctk.StringVar(value="未扫描")
        self._quests_dir: Path | None = None
        self._queue: queue.Queue[tuple[str, object]] = queue.Queue()
        self._build_ui()
        self.after(150, self._drain_queue)

    def _build_ui(self) -> None:
        ctk.set_appearance_mode("System")
        ctk.set_default_color_theme("blue")

        root = ctk.CTkFrame(self, corner_radius=0)
        root.pack(fill="both", expand=True, padx=18, pady=18)
        root.grid_columnconfigure(1, weight=1)

        title = ctk.CTkLabel(root, text="FTB Translater", font=ctk.CTkFont(size=24, weight="bold"))
        title.grid(row=0, column=0, columnspan=3, sticky="w", pady=(0, 18))

        ctk.CTkLabel(root, text="整合包目录").grid(row=1, column=0, sticky="w", padx=(0, 10), pady=8)
        ctk.CTkEntry(root, textvariable=self.selected_dir).grid(row=1, column=1, sticky="ew", pady=8)
        ctk.CTkButton(root, text="选择", width=88, command=self._choose_dir).grid(row=1, column=2, padx=(10, 0), pady=8)

        ctk.CTkLabel(root, text="DeepSeek Key").grid(row=2, column=0, sticky="w", padx=(0, 10), pady=8)
        ctk.CTkEntry(root, textvariable=self.api_key, show="*").grid(row=2, column=1, sticky="ew", pady=8)
        ctk.CTkButton(root, text="保存", width=88, command=self._save_key).grid(row=2, column=2, padx=(10, 0), pady=8)

        info = ctk.CTkFrame(root)
        info.grid(row=3, column=0, columnspan=3, sticky="ew", pady=(18, 8))
        info.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(info, textvariable=self.summary, anchor="w", justify="left").grid(row=0, column=0, sticky="ew", padx=14, pady=14)

        actions = ctk.CTkFrame(root, fg_color="transparent")
        actions.grid(row=4, column=0, columnspan=3, sticky="ew", pady=10)
        self.scan_button = ctk.CTkButton(actions, text="扫描", command=self._scan)
        self.scan_button.pack(side="left")
        self.translate_button = ctk.CTkButton(actions, text="开始汉化", command=self._start_translate, state="disabled")
        self.translate_button.pack(side="left", padx=10)

        self.progress = ctk.CTkProgressBar(root)
        self.progress.grid(row=5, column=0, columnspan=3, sticky="ew", pady=(18, 8))
        self.progress.set(0)

        self.log = ctk.CTkTextbox(root, height=150)
        self.log.grid(row=6, column=0, columnspan=3, sticky="nsew", pady=(10, 0))
        root.grid_rowconfigure(6, weight=1)

        ctk.CTkLabel(root, textvariable=self.status, anchor="w").grid(row=7, column=0, columnspan=3, sticky="ew", pady=(10, 0))

    def _choose_dir(self) -> None:
        directory = filedialog.askdirectory()
        if directory:
            self.selected_dir.set(directory)
            _log.info("User selected directory: %s", directory)
            self._scan()

    def _save_key(self) -> None:
        save_api_key(self.api_key.get())
        _log.info("Saved DEEPSEEK_API_KEY to .env")
        self._log("已保存 DEEPSEEK_API_KEY 到 .env。")

    def _scan(self) -> None:
        try:
            _log.info("Starting scan for: %s", self.selected_dir.get())
            quests_dir = resolve_quests_dir(Path(self.selected_dir.get()))
            mode = detect_source_mode(quests_dir)
            if mode == "lang":
                values = load_lang_snbt(source_lang_path(quests_dir))
                entry_count = len(values)
                source_label = str(source_lang_path(quests_dir))
                mode_label = "新版 lang/en_us.snbt"
                _log.info("Scan result: lang mode, %d entries from %s", entry_count, source_label)
            else:
                file_count, entry_count = count_chapter_segments(quests_dir)
                source_label = f"{quests_dir / 'chapters'}（{file_count} 个章节文件）"
                mode_label = "章节式 chapters/*.snbt"
                _log.info("Scan result: chapters mode, %d files, %d entries", file_count, entry_count)
            batches = estimate_batches(entry_count, AUTO_BATCH_MAX_ENTRIES)
        except Exception as exc:  # noqa: BLE001
            _log.error("Scan failed: %s", exc, exc_info=True)
            self._quests_dir = None
            self.translate_button.configure(state="disabled")
            self.summary.set("扫描失败")
            self.status.set(str(exc))
            self._log(f"扫描失败：{exc}")
            return

        self._quests_dir = quests_dir
        self.translate_button.configure(state="normal")
        self.summary.set(
            f"任务书目录：{quests_dir}\n"
            f"模式：{mode_label}\n"
            f"源：{source_label}\n"
            f"可翻译条目数：{entry_count}，自动切块约 {batches} 批，模型：{DEFAULT_MODEL}"
        )
        self.status.set("扫描完成，可以开始汉化。")
        self._log("扫描完成。")

    def _start_translate(self) -> None:
        _log.info("Translation requested by user")
        if self._quests_dir is None:
            self._scan()
        if self._quests_dir is None:
            return
        if not self.api_key.get().strip():
            messagebox.showerror("缺少 API Key", "请填写 DeepSeek API Key。")
            return
        mode = detect_source_mode(self._quests_dir)
        target = "lang/zh_cn.snbt" if mode == "lang" else "chapters/*.snbt"
        if not messagebox.askyesno(
            "确认覆盖写入",
            f"将先创建备份，然后覆盖写入 {target}。\n\n任务书目录：{self._quests_dir}\n\n是否继续？",
        ):
            self._log("已取消：未执行覆盖写入。")
            _log.info("User cancelled the overwrite confirmation")
            return
        self._save_key()
        self.scan_button.configure(state="disabled")
        self.translate_button.configure(state="disabled")
        self.progress.set(0)
        self.status.set("正在汉化...")
        _log.info("Starting translate worker thread (quests_dir=%s, mode=%s)", self._quests_dir, mode)
        thread = threading.Thread(target=self._translate_worker, daemon=True)
        thread.start()

    def _translate_worker(self) -> None:
        assert self._quests_dir is not None

        def progress(stage: str, done: int, total: int) -> None:
            self._queue.put(("progress", (stage, done, total)))

        def logger(message: str) -> None:
            self._queue.put(("log", message))

        try:
            _log.info("Calling translate_quests_auto with quests_dir=%s", self._quests_dir)
            report = translate_quests_auto(
                quests_dir=self._quests_dir,
                api_key=self.api_key.get(),
                progress=progress,
                logger=logger,
            )
            _log.info("Translation completed successfully: target=%s", report.target_file)
            self._queue.put(("done", report))
        except Exception as exc:  # noqa: BLE001
            _log.error("Translation worker failed: %s", exc, exc_info=True)
            self._queue.put(("error", exc))

    def _drain_queue(self) -> None:
        try:
            while True:
                kind, payload = self._queue.get_nowait()
                if kind == "progress":
                    stage, done, total = payload  # type: ignore[misc]
                    ratio = 1 if total == 0 else min(1, done / total)
                    self.progress.set(ratio)
                    self.status.set(f"{stage}: {done}/{total}")
                elif kind == "done":
                    report = payload
                    self.progress.set(1)
                    self.status.set("汉化完成。")
                    self._log(f"完成：写入 {report.target_file}")
                    self._log(f"备份：{report.backup_dir}")
                    self._log(f"缓存命中：{report.cache_hits}，失败：{len(report.failed_entries)}")
                    self.scan_button.configure(state="normal")
                    self.translate_button.configure(state="normal")
                elif kind == "log":
                    self._log(str(payload))
                elif kind == "error":
                    exc = payload
                    self.status.set(str(exc))
                    self._log(f"汉化失败：{exc}")
                    self.scan_button.configure(state="normal")
                    self.translate_button.configure(state="normal")
        except queue.Empty:
            pass
        self.after(150, self._drain_queue)

    def _log(self, text: str) -> None:
        """Log to both the GUI textbox and the file logger."""
        self.log.insert("end", text + "\n")
        self.log.see("end")
        _log.info("[GUI] %s", text)


def main() -> None:
    app = FtbTranslaterApp()
    app.mainloop()
