from __future__ import annotations

import queue
import threading
from pathlib import Path
from tkinter import filedialog, messagebox

import customtkinter as ctk

from ftb_translater.config import (
    BASE_URL_KEY,
    BATCH_SIZE_KEY,
    CONCURRENCY_KEY,
    ENV_KEY,
    MODEL_KEY,
    STYLE_KEY,
    load_api_key,
    load_config_values,
    save_config_values,
)
from ftb_translater.deepseek_client import DEFAULT_BASE_URL, DEFAULT_MODEL, DEFAULT_STYLE
from ftb_translater.chapters import count_chapter_segments
from ftb_translater.logger import get_logger, setup_logging
from ftb_translater.paths import detect_source_mode, resolve_quests_dir, source_lang_path
from ftb_translater.snbt import load_lang_snbt
from ftb_translater.translator import AUTO_BATCH_MAX_ENTRIES, AUTO_MAX_WORKERS, estimate_batches, translate_quests_auto

_log = get_logger(__name__)


class FtbTranslaterApp(ctk.CTk):
    def __init__(self):
        super().__init__()
        setup_logging()
        _log.info("FTB Translater starting up")
        self.title("FTB Translater")
        self.geometry("980x680")
        self.minsize(900, 620)

        config_values = load_config_values()
        self.selected_dir = ctk.StringVar()
        self.api_key = ctk.StringVar(value=config_values.get(ENV_KEY) or load_api_key())
        self.base_url = ctk.StringVar(value=config_values.get(BASE_URL_KEY) or DEFAULT_BASE_URL)
        self.model = ctk.StringVar(value=config_values.get(MODEL_KEY) or DEFAULT_MODEL)
        self.style = ctk.StringVar(value=config_values.get(STYLE_KEY) or DEFAULT_STYLE)
        self.batch_size = ctk.StringVar(value=config_values.get(BATCH_SIZE_KEY) or "auto")
        self.max_workers = ctk.StringVar(value=config_values.get(CONCURRENCY_KEY) or "auto")
        self.status = ctk.StringVar(value="请选择整合包目录，或它下面的 config/ftbquests/quests/lang/chapters 任一目录。")
        self.summary = ctk.StringVar(value="未扫描")
        self.stage = ctk.StringVar(value="准备就绪")
        self.progress_text = ctk.StringVar(value="等待扫描")
        self.settings_status = ctk.StringVar(value="API Key 会通过此界面保存和读取。")
        self._quests_dir: Path | None = None
        self._queue: queue.Queue[tuple[str, object]] = queue.Queue()
        self._key_visible = False
        self._step_labels: dict[str, ctk.CTkLabel] = {}
        self._nav_buttons: dict[str, ctk.CTkButton] = {}
        self._run_settings: dict[str, object] = {}
        self._build_ui()
        self._set_stage("idle")
        self.after(150, self._drain_queue)

    def _build_ui(self) -> None:
        ctk.set_appearance_mode("System")
        ctk.set_default_color_theme("blue")

        self.configure(fg_color=("#EEF2F7", "#111318"))
        self.grid_columnconfigure(1, weight=1)
        self.grid_rowconfigure(0, weight=1)

        sidebar = ctk.CTkFrame(self, width=248, corner_radius=0, fg_color=("#111827", "#0B0F14"))
        sidebar.grid(row=0, column=0, sticky="nsew")
        sidebar.grid_propagate(False)
        sidebar.grid_rowconfigure(11, weight=1)

        ctk.CTkLabel(
            sidebar,
            text="FTB\nTranslater",
            justify="left",
            font=ctk.CTkFont(size=30, weight="bold"),
            text_color="#F9FAFB",
        ).grid(row=0, column=0, sticky="ew", padx=24, pady=(30, 8))
        ctk.CTkLabel(
            sidebar,
            text="现代 FTB Quests 任务书汉化工具",
            justify="left",
            wraplength=180,
            text_color="#A7B0BF",
        ).grid(row=1, column=0, sticky="ew", padx=24, pady=(0, 30))

        for row, (key, text) in enumerate(
            [
                ("idle", "选择整合包"),
                ("scanned", "确认扫描结果"),
                ("running", "执行汉化"),
                ("done", "查看输出"),
            ],
            start=2,
        ):
            self._step_labels[key] = ctk.CTkLabel(
                sidebar,
                text=text,
                anchor="w",
                height=38,
                corner_radius=8,
                padx=14,
                font=ctk.CTkFont(size=14, weight="bold"),
                text_color="#DDE4EF",
                fg_color="transparent",
            )
            self._step_labels[key].grid(row=row, column=0, sticky="ew", padx=18, pady=4)

        ctk.CTkLabel(sidebar, text="目录", text_color="#7D8999", anchor="w").grid(
            row=6, column=0, sticky="ew", padx=24, pady=(22, 8)
        )
        self._nav_buttons["workbench"] = ctk.CTkButton(
            sidebar,
            text="工作台",
            anchor="w",
            height=38,
            command=lambda: self._show_view("workbench"),
        )
        self._nav_buttons["workbench"].grid(row=7, column=0, sticky="ew", padx=18, pady=4)
        self._nav_buttons["settings"] = ctk.CTkButton(
            sidebar,
            text="设置",
            anchor="w",
            height=38,
            command=lambda: self._show_view("settings"),
        )
        self._nav_buttons["settings"].grid(row=8, column=0, sticky="ew", padx=18, pady=4)

        ctk.CTkLabel(sidebar, text="外观", text_color="#7D8999", anchor="w").grid(
            row=9, column=0, sticky="ew", padx=24, pady=(22, 8)
        )
        self.appearance_segment = ctk.CTkSegmentedButton(
            sidebar,
            values=["系统", "浅色", "深色"],
            command=self._change_appearance,
            selected_color="#2563EB",
            selected_hover_color="#1D4ED8",
        )
        self.appearance_segment.grid(row=10, column=0, sticky="ew", padx=24)
        self.appearance_segment.set("系统")

        ctk.CTkLabel(sidebar, text="v0.1.0", text_color="#596579", anchor="w").grid(
            row=12, column=0, sticky="sw", padx=24, pady=24
        )

        main = ctk.CTkFrame(self, corner_radius=0, fg_color="transparent")
        main.grid(row=0, column=1, sticky="nsew", padx=24, pady=22)
        main.grid_columnconfigure(0, weight=1)
        main.grid_rowconfigure(4, weight=1)
        self.workbench_frame = main

        header = ctk.CTkFrame(main, fg_color="transparent")
        header.grid(row=0, column=0, sticky="ew", pady=(0, 16))
        header.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            header,
            text="任务书汉化工作台",
            anchor="w",
            font=ctk.CTkFont(size=28, weight="bold"),
        ).grid(row=0, column=0, sticky="ew")
        self.stage_badge = ctk.CTkLabel(
            header,
            textvariable=self.stage,
            height=34,
            corner_radius=17,
            padx=16,
            font=ctk.CTkFont(size=13, weight="bold"),
        )
        self.stage_badge.grid(row=0, column=1, sticky="e", padx=(18, 0))
        ctk.CTkLabel(
            header,
            textvariable=self.status,
            anchor="w",
            text_color=("#5B6676", "#AAB3C2"),
        ).grid(row=1, column=0, columnspan=2, sticky="ew", pady=(6, 0))

        source_panel = self._panel(main, "1. 选择整合包", "可以选择整合包根目录，也可以直接选择 quests、lang 或 chapters 目录。")
        source_panel.grid(row=1, column=0, sticky="ew", pady=(0, 12))
        source_panel.grid_columnconfigure(0, weight=1)
        ctk.CTkEntry(
            source_panel,
            textvariable=self.selected_dir,
            height=40,
            placeholder_text="选择整合包目录...",
        ).grid(row=2, column=0, sticky="ew", padx=18, pady=(0, 18))
        ctk.CTkButton(source_panel, text="选择目录", width=108, height=40, command=self._choose_dir).grid(
            row=2, column=1, padx=(0, 10), pady=(0, 18)
        )
        self.scan_button = ctk.CTkButton(source_panel, text="扫描", width=88, height=40, command=self._scan)
        self.scan_button.grid(row=2, column=2, padx=(0, 18), pady=(0, 18))

        summary_panel = self._panel(main, "2. 扫描结果", "扫描后会显示任务目录、文件模式、条目数量和预计批次数。")
        summary_panel.grid(row=2, column=0, sticky="ew", pady=(0, 12))
        summary_panel.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            summary_panel,
            textvariable=self.summary,
            anchor="w",
            justify="left",
            wraplength=660,
        ).grid(row=2, column=0, sticky="ew", padx=18, pady=(0, 18))

        run_panel = ctk.CTkFrame(main, corner_radius=14, border_width=1, border_color=("#D8DEE8", "#262B33"))
        run_panel.grid(row=3, column=0, sticky="ew", pady=(0, 12))
        run_panel.grid_columnconfigure(0, weight=1)
        self.progress = ctk.CTkProgressBar(run_panel, height=12)
        self.progress.grid(row=0, column=0, sticky="ew", padx=18, pady=(18, 8))
        self.progress.set(0)
        ctk.CTkLabel(run_panel, textvariable=self.progress_text, anchor="w", text_color=("#5B6676", "#AAB3C2")).grid(
            row=1, column=0, sticky="ew", padx=18, pady=(0, 18)
        )
        self.translate_button = ctk.CTkButton(
            run_panel,
            text="开始汉化",
            width=130,
            height=42,
            command=self._start_translate,
            state="disabled",
            font=ctk.CTkFont(size=15, weight="bold"),
        )
        self.translate_button.grid(row=0, column=1, rowspan=2, sticky="e", padx=18, pady=18)

        log_panel = self._panel(main, "运行日志", "这里会显示扫描、备份、翻译和写入过程。")
        log_panel.grid(row=4, column=0, sticky="nsew")
        log_panel.grid_columnconfigure(0, weight=1)
        log_panel.grid_rowconfigure(2, weight=1)
        self.log = ctk.CTkTextbox(log_panel, height=170, corner_radius=10)
        self.log.grid(row=2, column=0, sticky="nsew", padx=18, pady=(0, 18))

        self._build_settings_view()
        self._show_view("workbench")

    def _build_settings_view(self) -> None:
        settings = ctk.CTkFrame(self, corner_radius=0, fg_color="transparent")
        settings.grid(row=0, column=1, sticky="nsew", padx=24, pady=22)
        settings.grid_columnconfigure(0, weight=1)
        settings.grid_rowconfigure(1, weight=1)
        self.settings_frame = settings

        header = ctk.CTkFrame(settings, fg_color="transparent")
        header.grid(row=0, column=0, sticky="ew", pady=(0, 16))
        header.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            header,
            text="设置",
            anchor="w",
            font=ctk.CTkFont(size=28, weight="bold"),
        ).grid(row=0, column=0, sticky="ew")
        ctk.CTkButton(header, text="返回工作台", width=112, height=36, command=lambda: self._show_view("workbench")).grid(
            row=0, column=1, sticky="e", padx=(18, 0)
        )
        ctk.CTkLabel(
            header,
            text="这里放全局配置。修改后会影响之后的扫描和汉化任务。",
            anchor="w",
            text_color=("#5B6676", "#AAB3C2"),
        ).grid(row=1, column=0, columnspan=2, sticky="ew", pady=(6, 0))

        content = ctk.CTkScrollableFrame(settings, corner_radius=0, fg_color="transparent")
        content.grid(row=1, column=0, sticky="nsew")
        content.grid_columnconfigure(0, weight=1)

        api_panel = self._panel(content, "DeepSeek API Key", "用于调用 DeepSeek 翻译接口，保存后下次启动会自动读取。")
        api_panel.grid(row=0, column=0, sticky="ew", pady=(0, 12))
        api_panel.grid_columnconfigure(0, weight=1)
        self.key_entry = ctk.CTkEntry(api_panel, textvariable=self.api_key, show="*", height=40)
        self.key_entry.grid(row=2, column=0, sticky="ew", padx=18, pady=(0, 12))
        self.toggle_key_button = ctk.CTkButton(api_panel, text="显示", width=74, height=40, command=self._toggle_key_visibility)
        self.toggle_key_button.grid(row=2, column=1, padx=(0, 10), pady=(0, 12))
        ctk.CTkButton(api_panel, text="保存全部", width=96, height=40, command=self._save_settings).grid(
            row=2, column=2, padx=(0, 18), pady=(0, 12)
        )
        ctk.CTkLabel(
            api_panel,
            textvariable=self.settings_status,
            anchor="w",
            justify="left",
            wraplength=660,
            text_color=("#687386", "#AAB3C2"),
        ).grid(row=3, column=0, columnspan=3, sticky="ew", padx=18, pady=(0, 18))

        service_panel = self._panel(content, "DeepSeek 服务", "默认使用官方 DeepSeek 兼容 OpenAI 接口，也可以改成兼容服务地址。")
        service_panel.grid(row=1, column=0, sticky="ew", pady=(0, 12))
        service_panel.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(service_panel, text="API 地址", anchor="w").grid(
            row=2, column=0, sticky="ew", padx=18, pady=(0, 6)
        )
        ctk.CTkEntry(service_panel, textvariable=self.base_url, height=40).grid(
            row=3, column=0, columnspan=2, sticky="ew", padx=18, pady=(0, 14)
        )
        ctk.CTkLabel(service_panel, text="模型", anchor="w").grid(
            row=4, column=0, sticky="ew", padx=18, pady=(0, 6)
        )
        ctk.CTkEntry(service_panel, textvariable=self.model, height=40).grid(
            row=5, column=0, sticky="ew", padx=18, pady=(0, 18)
        )
        ctk.CTkButton(service_panel, text="恢复默认", width=96, height=40, command=self._reset_service_settings).grid(
            row=5, column=1, sticky="e", padx=(0, 18), pady=(0, 18)
        )

        translate_panel = self._panel(content, "翻译参数", "批大小和并发可填 auto 使用自动策略，也可填正整数手动控制。")
        translate_panel.grid(row=2, column=0, sticky="ew", pady=(0, 12))
        translate_panel.grid_columnconfigure(0, weight=1)
        translate_panel.grid_columnconfigure(1, weight=1)
        ctk.CTkLabel(translate_panel, text="翻译风格", anchor="w").grid(
            row=2, column=0, columnspan=2, sticky="ew", padx=18, pady=(0, 6)
        )
        self.style_text = ctk.CTkTextbox(translate_panel, height=76, corner_radius=10)
        self.style_text.grid(row=3, column=0, columnspan=2, sticky="ew", padx=18, pady=(0, 14))
        self.style_text.insert("1.0", self.style.get())
        ctk.CTkLabel(translate_panel, text="批大小", anchor="w").grid(
            row=4, column=0, sticky="ew", padx=18, pady=(0, 6)
        )
        ctk.CTkLabel(translate_panel, text="并发数", anchor="w").grid(
            row=4, column=1, sticky="ew", padx=(0, 18), pady=(0, 6)
        )
        ctk.CTkEntry(translate_panel, textvariable=self.batch_size, height=40, placeholder_text="auto").grid(
            row=5, column=0, sticky="ew", padx=18, pady=(0, 18)
        )
        ctk.CTkEntry(translate_panel, textvariable=self.max_workers, height=40, placeholder_text="auto").grid(
            row=5, column=1, sticky="ew", padx=(0, 18), pady=(0, 18)
        )
        ctk.CTkLabel(
            translate_panel,
            text=f"auto 批大小：最多 {AUTO_BATCH_MAX_ENTRIES} 条 / {AUTO_MAX_WORKERS} 个自动并发上限",
            anchor="w",
            justify="left",
            text_color=("#687386", "#AAB3C2"),
        ).grid(row=6, column=0, columnspan=2, sticky="ew", padx=18, pady=(0, 18))

    def _panel(self, parent: ctk.CTkFrame, title: str, description: str) -> ctk.CTkFrame:
        panel = ctk.CTkFrame(parent, corner_radius=14, border_width=1, border_color=("#D8DEE8", "#262B33"))
        ctk.CTkLabel(panel, text=title, anchor="w", font=ctk.CTkFont(size=16, weight="bold")).grid(
            row=0, column=0, columnspan=3, sticky="ew", padx=18, pady=(16, 2)
        )
        ctk.CTkLabel(panel, text=description, anchor="w", text_color=("#687386", "#AAB3C2")).grid(
            row=1, column=0, columnspan=3, sticky="ew", padx=18, pady=(0, 14)
        )
        return panel

    def _show_view(self, view: str) -> None:
        if view == "settings":
            self.workbench_frame.grid_remove()
            self.settings_frame.grid()
        else:
            self.settings_frame.grid_remove()
            self.workbench_frame.grid()
            view = "workbench"
        self._set_nav_state(view)

    def _set_nav_state(self, active_view: str) -> None:
        for view, button in self._nav_buttons.items():
            if view == active_view:
                button.configure(fg_color="#2563EB", hover_color="#1D4ED8", text_color="#FFFFFF")
            else:
                button.configure(fg_color="transparent", hover_color="#1F2937", text_color="#DDE4EF")

    def _change_appearance(self, value: str) -> None:
        modes = {"系统": "System", "浅色": "Light", "深色": "Dark"}
        ctk.set_appearance_mode(modes.get(value, "System"))

    def _toggle_key_visibility(self) -> None:
        self._key_visible = not self._key_visible
        self.key_entry.configure(show="" if self._key_visible else "*")
        self.toggle_key_button.configure(text="隐藏" if self._key_visible else "显示")

    def _reset_service_settings(self) -> None:
        self.base_url.set(DEFAULT_BASE_URL)
        self.model.set(DEFAULT_MODEL)
        self.settings_status.set("已恢复默认 DeepSeek 服务参数，点击保存全部后生效。")

    def _current_style(self) -> str:
        text = self.style_text.get("1.0", "end").strip()
        return text or DEFAULT_STYLE

    def _parse_optional_positive_int(self, value: str, label: str) -> int | None:
        stripped = value.strip()
        if not stripped or stripped.lower() == "auto":
            return None
        try:
            parsed = int(stripped)
        except ValueError as exc:
            raise ValueError(f"{label} 必须填写 auto 或正整数。") from exc
        if parsed <= 0:
            raise ValueError(f"{label} 必须大于 0。")
        return parsed

    def _effective_batch_size(self) -> int | None:
        return self._parse_optional_positive_int(self.batch_size.get(), "批大小")

    def _effective_max_workers(self) -> int | None:
        return self._parse_optional_positive_int(self.max_workers.get(), "并发数")

    def _save_settings(self) -> None:
        try:
            self._effective_batch_size()
            self._effective_max_workers()
        except ValueError as exc:
            self.settings_status.set(str(exc))
            messagebox.showerror("配置无效", str(exc))
            return

        self.style.set(self._current_style())
        save_config_values(
            {
                ENV_KEY: self.api_key.get(),
                BASE_URL_KEY: self.base_url.get() or DEFAULT_BASE_URL,
                MODEL_KEY: self.model.get() or DEFAULT_MODEL,
                STYLE_KEY: self.style.get() or DEFAULT_STYLE,
                BATCH_SIZE_KEY: self.batch_size.get() or "auto",
                CONCURRENCY_KEY: self.max_workers.get() or "auto",
            }
        )
        self.settings_status.set("已保存全部设置。")
        _log.info("Saved app settings")
        self._log("已保存全部设置。")

    def _set_stage(self, stage: str) -> None:
        labels = {
            "idle": ("准备就绪", "#2563EB"),
            "scanned": ("已完成扫描", "#059669"),
            "running": ("正在汉化", "#D97706"),
            "done": ("汉化完成", "#059669"),
            "error": ("需要处理", "#DC2626"),
        }
        text, color = labels.get(stage, labels["idle"])
        self.stage.set(text)
        self.stage_badge.configure(fg_color=color, text_color="#FFFFFF")

        active_order = ["idle", "scanned", "running", "done"]
        active_index = active_order.index(stage) if stage in active_order else 0
        for index, key in enumerate(active_order):
            label = self._step_labels.get(key)
            if label is None:
                continue
            if index <= active_index and stage != "error":
                label.configure(fg_color="#1D4ED8", text_color="#FFFFFF")
            else:
                label.configure(fg_color="transparent", text_color="#DDE4EF")

    def _choose_dir(self) -> None:
        directory = filedialog.askdirectory()
        if directory:
            self.selected_dir.set(directory)
            _log.info("User selected directory: %s", directory)
            self._scan()

    def _save_key(self) -> None:
        self._save_settings()

    def _scan(self) -> None:
        try:
            _log.info("Starting scan for: %s", self.selected_dir.get())
            batch_size = self._effective_batch_size()
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
            effective_batch_size = batch_size or AUTO_BATCH_MAX_ENTRIES
            batches = estimate_batches(entry_count, effective_batch_size)
        except Exception as exc:  # noqa: BLE001
            _log.error("Scan failed: %s", exc, exc_info=True)
            self._quests_dir = None
            self.translate_button.configure(state="disabled")
            self.summary.set("扫描失败")
            self.status.set(str(exc))
            self.progress_text.set("扫描失败")
            self._set_stage("error")
            self._log(f"扫描失败：{exc}")
            return

        self._quests_dir = quests_dir
        self.translate_button.configure(state="normal")
        self.summary.set(
            f"任务书目录：{quests_dir}\n"
            f"模式：{mode_label}\n"
            f"源：{source_label}\n"
            f"可翻译条目数：{entry_count}，预计 {batches} 批，模型：{self.model.get() or DEFAULT_MODEL}"
        )
        self.status.set("扫描完成，可以开始汉化。")
        self.progress_text.set(f"扫描完成：{entry_count} 条，预计 {batches} 批")
        self._set_stage("scanned")
        self._log("扫描完成。")

    def _start_translate(self) -> None:
        _log.info("Translation requested by user")
        if self._quests_dir is None:
            self._scan()
        if self._quests_dir is None:
            return
        try:
            batch_size = self._effective_batch_size()
            max_workers = self._effective_max_workers()
        except ValueError as exc:
            self._show_view("settings")
            self.settings_status.set(str(exc))
            messagebox.showerror("配置无效", str(exc))
            return
        if not self.api_key.get().strip():
            self._show_view("settings")
            self.settings_status.set("请先填写 API Key，然后点击保存全部。")
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
        self._save_settings()
        self._run_settings = {
            "api_key": self.api_key.get(),
            "batch_size": batch_size,
            "model": self.model.get() or DEFAULT_MODEL,
            "style": self._current_style(),
            "base_url": self.base_url.get() or DEFAULT_BASE_URL,
            "max_workers": max_workers,
        }
        self.scan_button.configure(state="disabled")
        self.translate_button.configure(state="disabled")
        self.progress.set(0)
        self.status.set("正在汉化...")
        self.progress_text.set("正在准备翻译任务...")
        self._set_stage("running")
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
                api_key=str(self._run_settings["api_key"]),
                batch_size=self._run_settings["batch_size"],  # type: ignore[arg-type]
                model=str(self._run_settings["model"]),
                style=str(self._run_settings["style"]),
                base_url=str(self._run_settings["base_url"]),
                max_workers=self._run_settings["max_workers"],  # type: ignore[arg-type]
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
                    self.progress_text.set(f"{stage}：{done}/{total}")
                elif kind == "done":
                    report = payload
                    self.progress.set(1)
                    self.status.set("汉化完成。")
                    self.progress_text.set("汉化完成，可以查看日志和输出文件。")
                    self._set_stage("done")
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
                    self.progress_text.set("汉化失败，请查看日志。")
                    self._set_stage("error")
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
