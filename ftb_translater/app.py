from __future__ import annotations

import queue
import threading
from collections import OrderedDict
from pathlib import Path
from tkinter import filedialog, messagebox
from typing import Literal, TypeAlias, TypedDict, cast

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
from ftb_translater.deepseek_client import DEFAULT_BASE_URL, DEFAULT_MODEL, DEFAULT_STYLE, DeepSeekTranslator
from ftb_translater.format_guard import protect_text, restore_text, preserved_token_warnings
from ftb_translater.report import TranslationReport
from ftb_translater.chapters import count_chapter_segments, replace_chapter_segments
from ftb_translater.logger import get_logger, setup_logging
from ftb_translater.paths import detect_source_mode, resolve_quests_dir, source_lang_path
from ftb_translater.snbt import load_lang_snbt, write_lang_snbt
from ftb_translater.translator import AUTO_BATCH_MAX_ENTRIES, AUTO_MAX_WORKERS, estimate_batches, translate_quests_auto

_log = get_logger(__name__)


class _RunSettings(TypedDict):
    api_key: str
    batch_size: int | None
    model: str
    style: str
    base_url: str
    max_workers: int | None


class _ReviewEntryData(TypedDict):
    source: str
    failed: str
    textbox: ctk.CTkTextbox
    status_label: ctk.CTkLabel
    retrans_btn: ctk.CTkButton
    frame: ctk.CTkFrame


_ProgressPayload: TypeAlias = tuple[str, int, int]
_AppQueueItem: TypeAlias = (
    tuple[Literal["progress"], _ProgressPayload]
    | tuple[Literal["log"], str]
    | tuple[Literal["done"], TranslationReport]
    | tuple[Literal["error"], Exception]
)


class FtbTranslaterApp(ctk.CTk):
    def __init__(self):
        super().__init__()
        setup_logging()
        _log.info("FTB Translater starting up")
        self.title("FTB Translater")
        self.geometry("1120x780")
        self.minsize(980, 700)

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
        self._queue: queue.Queue[_AppQueueItem] = queue.Queue()
        self._key_visible = False
        self._step_labels: dict[str, ctk.CTkLabel] = {}
        self._nav_buttons: dict[str, ctk.CTkButton] = {}
        self._run_settings: _RunSettings | None = None
        self._review_report: TranslationReport | None = None
        self._review_data: dict[str, _ReviewEntryData] = {}
        self._build_ui()
        self._set_stage("idle")
        self.after(150, self._drain_queue)

    def _build_ui(self) -> None:
        ctk.set_appearance_mode("System")
        ctk.set_default_color_theme("blue")

        self.configure(fg_color=("#F8FAFB", "#0D1117"))
        self.grid_columnconfigure(0, weight=1)
        self.grid_rowconfigure(1, weight=1)

        topbar = ctk.CTkFrame(
            self,
            height=56,
            corner_radius=0,
            fg_color=("#FFFFFF", "#161B22"),
            border_width=0,
        )
        topbar.grid(row=0, column=0, sticky="ew")
        topbar.grid_columnconfigure(1, weight=1)
        topbar.grid_rowconfigure(0, weight=1)
        topbar.grid_propagate(False)

        brand = ctk.CTkFrame(topbar, fg_color="transparent")
        brand.grid(row=0, column=0, sticky="w", padx=24)
        ctk.CTkLabel(
            brand,
            text="FTB Translater",
            anchor="w",
            font=ctk.CTkFont(size=18, weight="bold"),
            text_color=("#1F2328", "#E6EDF3"),
        ).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(
            brand,
            text="任务书汉化",
            anchor="w",
            text_color=("#656D76", "#8B949E"),
            font=ctk.CTkFont(size=12),
        ).grid(row=0, column=1, sticky="w", padx=(10, 0))

        right_bar = ctk.CTkFrame(topbar, fg_color="transparent")
        right_bar.grid(row=0, column=2, sticky="e", padx=24)
        self._nav_buttons["settings"] = ctk.CTkButton(
            right_bar,
            text="⚙ 设置",
            width=72,
            height=32,
            corner_radius=8,
            command=lambda: self._show_view("settings"),
            font=ctk.CTkFont(size=13),
            fg_color=("#F3F4F6", "#21262D"),
            hover_color=("#E5E7EB", "#30363D"),
            text_color=("#374151", "#C9D1D9"),
        )
        self._nav_buttons["settings"].grid(row=0, column=0, padx=(0, 10))
        self.appearance_segment = ctk.CTkSegmentedButton(
            right_bar,
            values=["系统", "浅色", "深色"],
            command=self._change_appearance,
            selected_color="#2563EB",
            selected_hover_color="#1D4ED8",
            height=30,
            font=ctk.CTkFont(size=12),
        )
        self.appearance_segment.grid(row=0, column=1)
        self.appearance_segment.set("系统")

        main = ctk.CTkFrame(self, corner_radius=0, fg_color="transparent")
        main.grid(row=1, column=0, sticky="nsew", padx=32, pady=(20, 24))
        main.grid_columnconfigure(0, weight=1)
        main.grid_rowconfigure(4, weight=1)
        self.workbench_frame = main

        header = ctk.CTkFrame(main, fg_color="transparent")
        header.grid(row=0, column=0, sticky="ew", pady=(0, 20))
        header.grid_columnconfigure(0, weight=1)

        title_row = ctk.CTkFrame(header, fg_color="transparent")
        title_row.grid(row=0, column=0, sticky="ew")
        title_row.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            title_row,
            text="工作台",
            anchor="w",
            font=ctk.CTkFont(size=24, weight="bold"),
            text_color=("#1F2328", "#E6EDF3"),
        ).grid(row=0, column=0, sticky="w")
        self.stage_badge = ctk.CTkLabel(
            title_row,
            textvariable=self.stage,
            height=28,
            corner_radius=14,
            padx=14,
            font=ctk.CTkFont(size=12, weight="bold"),
        )
        self.stage_badge.grid(row=0, column=1, sticky="e")

        ctk.CTkLabel(
            header,
            textvariable=self.status,
            anchor="w",
            text_color=("#656D76", "#8B949E"),
            font=ctk.CTkFont(size=13),
        ).grid(row=1, column=0, sticky="ew", pady=(6, 0))

        stepper = ctk.CTkFrame(header, fg_color="transparent", height=36)
        stepper.grid(row=2, column=0, sticky="ew", pady=(12, 0))
        for col in range(4):
            stepper.grid_columnconfigure(col, weight=0)
        stepper.grid_columnconfigure(4, weight=1)
        for col, (key, text) in enumerate(
            [
                ("idle", "① 选择"),
                ("scanned", "② 扫描"),
                ("running", "③ 翻译"),
                ("done", "④ 完成"),
            ]
        ):
            self._step_labels[key] = ctk.CTkLabel(
                stepper,
                text=text,
                height=28,
                corner_radius=6,
                padx=10,
                font=ctk.CTkFont(size=12),
                text_color=("#656D76", "#8B949E"),
                fg_color=("#F3F4F6", "#1C2128"),
            )
            self._step_labels[key].grid(row=0, column=col, padx=(0, 6))

        source_panel = self._panel(main, "选择整合包", "选择整合包根目录，或直接选择 quests / lang / chapters 目录")
        source_panel.grid(row=1, column=0, sticky="ew", pady=(0, 10))
        source_panel.grid_columnconfigure(0, weight=1)
        ctk.CTkEntry(
            source_panel,
            textvariable=self.selected_dir,
            height=36,
            corner_radius=8,
            placeholder_text="选择整合包目录...",
            border_width=1,
            border_color=("#D0D7DE", "#30363D"),
        ).grid(row=2, column=0, sticky="ew", padx=16, pady=(0, 14))
        ctk.CTkButton(
            source_panel, text="选择目录", width=90, height=36,
            corner_radius=8, command=self._choose_dir,
            fg_color=("#2563EB", "#2563EB"), hover_color=("#1D4ED8", "#1D4ED8"),
        ).grid(row=2, column=1, padx=(0, 8), pady=(0, 14))
        self.scan_button = ctk.CTkButton(
            source_panel, text="扫描", width=64, height=36,
            corner_radius=8, command=self._scan,
            fg_color=("#F3F4F6", "#21262D"), hover_color=("#E5E7EB", "#30363D"),
            text_color=("#374151", "#C9D1D9"),
        )
        self.scan_button.grid(row=2, column=2, padx=(0, 16), pady=(0, 14))

        summary_panel = self._panel(main, "扫描结果", "扫描后显示目录、模式、条目数和预计批次")
        summary_panel.grid(row=2, column=0, sticky="ew", pady=(0, 10))
        summary_panel.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            summary_panel,
            textvariable=self.summary,
            anchor="w",
            justify="left",
            wraplength=700,
            font=ctk.CTkFont(size=13),
            text_color=("#374151", "#C9D1D9"),
        ).grid(row=2, column=0, sticky="ew", padx=16, pady=(0, 14))

        run_panel = ctk.CTkFrame(
            main,
            corner_radius=12,
            border_width=1,
            fg_color=("#FFFFFF", "#161B22"),
            border_color=("#D0D7DE", "#30363D"),
        )
        run_panel.grid(row=3, column=0, sticky="ew", pady=(0, 10))
        run_panel.grid_columnconfigure(0, weight=1)
        self.progress = ctk.CTkProgressBar(
            run_panel, height=8, corner_radius=4,
            progress_color="#2563EB",
        )
        self.progress.grid(row=0, column=0, sticky="ew", padx=16, pady=(16, 6))
        self.progress.set(0)
        ctk.CTkLabel(
            run_panel, textvariable=self.progress_text, anchor="w",
            text_color=("#656D76", "#8B949E"), font=ctk.CTkFont(size=12),
        ).grid(row=1, column=0, sticky="ew", padx=16, pady=(0, 16))
        self.translate_button = ctk.CTkButton(
            run_panel,
            text="开始汉化",
            width=110,
            height=36,
            corner_radius=8,
            command=self._start_translate,
            state="disabled",
            font=ctk.CTkFont(size=14, weight="bold"),
            fg_color=("#16A34A", "#238636"),
            hover_color=("#15803D", "#2EA043"),
        )
        self.translate_button.grid(row=0, column=1, rowspan=2, sticky="e", padx=16, pady=16)

        bottom = ctk.CTkFrame(main, corner_radius=0, fg_color="transparent")
        bottom.grid(row=4, column=0, sticky="nsew")
        bottom.grid_columnconfigure(0, weight=1)
        bottom.grid_rowconfigure(0, weight=1)

        log_panel = self._panel(bottom, "日志", "扫描、备份、翻译和写入的实时输出")
        log_panel.grid(row=0, column=0, sticky="nsew")
        log_panel.grid_columnconfigure(0, weight=1)
        log_panel.grid_rowconfigure(2, weight=1)
        self.log = ctk.CTkTextbox(
            log_panel, height=140, corner_radius=8,
            font=ctk.CTkFont(size=12, family="Menlo"),
            fg_color=("#F6F8FA", "#0D1117"),
            text_color=("#1F2328", "#C9D1D9"),
        )
        self.log.grid(row=2, column=0, sticky="nsew", padx=16, pady=(0, 14))

        review_panel = ctk.CTkFrame(
            bottom, corner_radius=12, border_width=1,
            fg_color=("#FFFBEB", "#1C1917"),
            border_color=("#F59E0B", "#92400E"),
        )
        review_panel.grid(row=1, column=0, sticky="nsew", pady=(10, 0))
        review_panel.grid_columnconfigure(0, weight=1)
        review_panel.grid_columnconfigure(1, weight=0)
        review_panel.grid_rowconfigure(3, weight=1)
        review_panel.grid_remove()
        self.review_panel = review_panel

        self._review_title = ctk.StringVar(value="人工处理")
        ctk.CTkLabel(
            review_panel, textvariable=self._review_title, anchor="w",
            font=ctk.CTkFont(size=15, weight="bold"),
            text_color=("#92400E", "#FCD34D"),
        ).grid(row=0, column=0, sticky="ew", padx=16, pady=(14, 2))

        self._review_badge = ctk.CTkLabel(
            review_panel, text="", height=24, corner_radius=12, padx=10,
            fg_color=("#F59E0B", "#B45309"), text_color="#FFFFFF",
            font=ctk.CTkFont(size=11, weight="bold"),
        )
        self._review_badge.grid(row=0, column=1, sticky="e", padx=(0, 16), pady=(14, 2))

        self._review_subtitle = ctk.CTkLabel(
            review_panel, text="", anchor="w",
            text_color=("#78716C", "#A8A29E"),
            wraplength=760, font=ctk.CTkFont(size=12),
        )
        self._review_subtitle.grid(row=1, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 6))

        action_bar = ctk.CTkFrame(review_panel, fg_color="transparent")
        action_bar.grid(row=2, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 6))
        action_bar.grid_columnconfigure(0, weight=1)
        self._retranslate_all_btn = ctk.CTkButton(
            action_bar, text="全部重新翻译", width=120, height=30,
            corner_radius=8, command=self._retranslate_all_review,
            fg_color=("#F59E0B", "#B45309"), hover_color=("#D97706", "#92400E"),
        )
        self._retranslate_all_btn.grid(row=0, column=1, padx=(0, 6), sticky="e")
        self._ignore_all_btn = ctk.CTkButton(
            action_bar, text="全部忽略", width=90, height=30,
            corner_radius=8,
            fg_color=("#6B7280", "#4B5563"), hover_color=("#4B5563", "#374151"),
            command=self._ignore_all_review,
        )
        self._ignore_all_btn.grid(row=0, column=2, sticky="e")

        self._review_scroll = ctk.CTkScrollableFrame(
            review_panel, corner_radius=8, border_width=1,
            fg_color=("#FFFFFF", "#0D1117"),
            border_color=("#E5E7EB", "#30363D"),
        )
        self._review_scroll.grid(row=3, column=0, columnspan=2, sticky="nsew", padx=16, pady=(0, 14))
        self._review_scroll.grid_columnconfigure(0, weight=1)

        self._build_settings_view()
        self._show_view("workbench")

    def _build_settings_view(self) -> None:
        settings = ctk.CTkFrame(self, corner_radius=0, fg_color="transparent")
        settings.grid(row=1, column=0, sticky="nsew", padx=32, pady=(20, 24))
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
            font=ctk.CTkFont(size=24, weight="bold"),
            text_color=("#1F2328", "#E6EDF3"),
        ).grid(row=0, column=0, sticky="ew")
        ctk.CTkButton(
            header, text="← 返回工作台", width=110, height=32,
            corner_radius=8, command=lambda: self._show_view("workbench"),
            fg_color=("#F3F4F6", "#21262D"), hover_color=("#E5E7EB", "#30363D"),
            text_color=("#374151", "#C9D1D9"),
        ).grid(row=0, column=1, sticky="e")
        ctk.CTkLabel(
            header,
            text="修改后会影响之后的扫描和汉化任务",
            anchor="w",
            text_color=("#656D76", "#8B949E"),
            font=ctk.CTkFont(size=13),
        ).grid(row=1, column=0, columnspan=2, sticky="ew", pady=(6, 0))

        content = ctk.CTkScrollableFrame(settings, corner_radius=0, fg_color="transparent")
        content.grid(row=1, column=0, sticky="nsew")
        content.grid_columnconfigure(0, weight=1)

        api_panel = self._panel(content, "API Key", "用于调用翻译接口，保存后下次启动自动读取")
        api_panel.grid(row=0, column=0, sticky="ew", pady=(0, 10))
        api_panel.grid_columnconfigure(0, weight=1)
        self.key_entry = ctk.CTkEntry(
            api_panel, textvariable=self.api_key, show="*", height=36,
            corner_radius=8, border_width=1, border_color=("#D0D7DE", "#30363D"),
        )
        self.key_entry.grid(row=2, column=0, sticky="ew", padx=16, pady=(0, 10))
        self.toggle_key_button = ctk.CTkButton(
            api_panel, text="显示", width=60, height=36, corner_radius=8,
            command=self._toggle_key_visibility,
            fg_color=("#F3F4F6", "#21262D"), hover_color=("#E5E7EB", "#30363D"),
            text_color=("#374151", "#C9D1D9"),
        )
        self.toggle_key_button.grid(row=2, column=1, padx=(0, 8), pady=(0, 10))
        ctk.CTkButton(
            api_panel, text="保存全部", width=80, height=36, corner_radius=8,
            command=self._save_settings,
            fg_color=("#2563EB", "#2563EB"), hover_color=("#1D4ED8", "#1D4ED8"),
        ).grid(row=2, column=2, padx=(0, 16), pady=(0, 10))
        ctk.CTkLabel(
            api_panel,
            textvariable=self.settings_status,
            anchor="w",
            justify="left",
            wraplength=660,
            text_color=("#656D76", "#8B949E"),
            font=ctk.CTkFont(size=12),
        ).grid(row=3, column=0, columnspan=3, sticky="ew", padx=16, pady=(0, 14))

        service_panel = self._panel(content, "翻译服务", "默认使用 DeepSeek 兼容 OpenAI 接口，可替换为其他兼容服务")
        service_panel.grid(row=1, column=0, sticky="ew", pady=(0, 10))
        service_panel.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            service_panel, text="API 地址", anchor="w",
            font=ctk.CTkFont(size=12), text_color=("#374151", "#C9D1D9"),
        ).grid(row=2, column=0, sticky="ew", padx=16, pady=(0, 4))
        ctk.CTkEntry(
            service_panel, textvariable=self.base_url, height=36,
            corner_radius=8, border_width=1, border_color=("#D0D7DE", "#30363D"),
        ).grid(row=3, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 12))
        ctk.CTkLabel(
            service_panel, text="模型", anchor="w",
            font=ctk.CTkFont(size=12), text_color=("#374151", "#C9D1D9"),
        ).grid(row=4, column=0, sticky="ew", padx=16, pady=(0, 4))
        ctk.CTkEntry(
            service_panel, textvariable=self.model, height=36,
            corner_radius=8, border_width=1, border_color=("#D0D7DE", "#30363D"),
        ).grid(row=5, column=0, sticky="ew", padx=16, pady=(0, 14))
        ctk.CTkButton(
            service_panel, text="恢复默认", width=80, height=32, corner_radius=8,
            command=self._reset_service_settings,
            fg_color=("#F3F4F6", "#21262D"), hover_color=("#E5E7EB", "#30363D"),
            text_color=("#374151", "#C9D1D9"),
        ).grid(row=5, column=1, sticky="e", padx=(0, 16), pady=(0, 14))

        translate_panel = self._panel(content, "翻译参数", "批大小和并发可填 auto 使用自动策略，或填正整数手动控制")
        translate_panel.grid(row=2, column=0, sticky="ew", pady=(0, 10))
        translate_panel.grid_columnconfigure(0, weight=1)
        translate_panel.grid_columnconfigure(1, weight=1)
        ctk.CTkLabel(
            translate_panel, text="翻译风格", anchor="w",
            font=ctk.CTkFont(size=12), text_color=("#374151", "#C9D1D9"),
        ).grid(row=2, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 4))
        self.style_text = ctk.CTkTextbox(
            translate_panel, height=68, corner_radius=8,
            font=ctk.CTkFont(size=13),
            fg_color=("#F6F8FA", "#0D1117"),
            border_width=1, border_color=("#D0D7DE", "#30363D"),
        )
        self.style_text.grid(row=3, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 12))
        self.style_text.insert("1.0", self.style.get())
        ctk.CTkLabel(
            translate_panel, text="批大小", anchor="w",
            font=ctk.CTkFont(size=12), text_color=("#374151", "#C9D1D9"),
        ).grid(row=4, column=0, sticky="ew", padx=16, pady=(0, 4))
        ctk.CTkLabel(
            translate_panel, text="并发数", anchor="w",
            font=ctk.CTkFont(size=12), text_color=("#374151", "#C9D1D9"),
        ).grid(row=4, column=1, sticky="ew", padx=(0, 16), pady=(0, 4))
        ctk.CTkEntry(
            translate_panel, textvariable=self.batch_size, height=36,
            corner_radius=8, placeholder_text="auto",
            border_width=1, border_color=("#D0D7DE", "#30363D"),
        ).grid(row=5, column=0, sticky="ew", padx=16, pady=(0, 14))
        ctk.CTkEntry(
            translate_panel, textvariable=self.max_workers, height=36,
            corner_radius=8, placeholder_text="auto",
            border_width=1, border_color=("#D0D7DE", "#30363D"),
        ).grid(row=5, column=1, sticky="ew", padx=(0, 16), pady=(0, 14))
        ctk.CTkLabel(
            translate_panel,
            text=f"auto: 最多 {AUTO_BATCH_MAX_ENTRIES} 条/批，{AUTO_MAX_WORKERS} 并发",
            anchor="w",
            text_color=("#656D76", "#8B949E"),
            font=ctk.CTkFont(size=11),
        ).grid(row=6, column=0, columnspan=2, sticky="ew", padx=16, pady=(0, 14))

    def _show_review_entries(self) -> None:
        for widget in self._review_scroll.winfo_children():
            widget.destroy()
        self._review_data.clear()

        report = self._review_report
        if report is None:
            self._hide_review_panel()
            return

        entries = self._review_entries(report)
        if not entries:
            self._hide_review_panel()
            return

        self.review_panel.grid()
        self.review_panel.master.grid_rowconfigure(1, weight=1)
        self._review_title.set(f"人工处理  —  {len(entries)} 条需要确认的映射")
        self._review_badge.configure(text=f"待处理 {len(entries)}")

        self._review_subtitle.configure(
            text="这些条目因为 API 失败或格式保护被保留为原文。可以逐条编辑后保存，也可以重新翻译或忽略。"
        )

        for idx, (key, warning_list) in enumerate(entries):
            ft = report.failed_translations.get(key, {})
            source = ft.get("source", key)
            failed = ft.get("failed", "")

            card = ctk.CTkFrame(
                self._review_scroll, corner_radius=10,
                fg_color=("#FFFFFF", "#161B22"),
                border_width=1, border_color=("#D0D7DE", "#30363D"),
            )
            card.grid(row=idx, column=0, sticky="ew", pady=(0, 8))
            card.grid_columnconfigure(0, weight=1)
            card.grid_columnconfigure(1, weight=1)

            key_row = ctk.CTkFrame(card, fg_color="transparent")
            key_row.grid(row=0, column=0, columnspan=2, sticky="ew", padx=12, pady=(10, 6))
            key_row.grid_columnconfigure(0, weight=1)
            ctk.CTkLabel(
                key_row, text=key, anchor="w",
                font=ctk.CTkFont(size=11, family="Menlo"),
                text_color=("#656D76", "#8B949E"),
            ).grid(row=0, column=0, sticky="ew")
            ctk.CTkLabel(
                key_row, text="待确认", height=20, corner_radius=10, padx=8,
                fg_color=("#FEF3C7", "#451A03"), text_color=("#92400E", "#FCD34D"),
                font=ctk.CTkFont(size=10, weight="bold"),
            ).grid(row=0, column=1, sticky="e")

            ctk.CTkLabel(
                card, text="原文", anchor="w",
                font=ctk.CTkFont(size=12, weight="bold"),
                text_color=("#374151", "#C9D1D9"),
            ).grid(row=1, column=0, sticky="w", padx=12, pady=(0, 2))
            ctk.CTkLabel(
                card, text="翻译（可编辑）", anchor="w",
                font=ctk.CTkFont(size=12, weight="bold"),
                text_color=("#374151", "#C9D1D9"),
            ).grid(row=1, column=1, sticky="w", padx=12, pady=(0, 2))

            src_tb = ctk.CTkTextbox(
                card, height=90, corner_radius=6,
                fg_color=("#F6F8FA", "#0D1117"),
                border_width=1, border_color=("#D0D7DE", "#30363D"),
                font=ctk.CTkFont(size=12),
            )
            src_tb.grid(row=2, column=0, sticky="nsew", padx=(12, 6), pady=(0, 4))
            src_tb.insert("1.0", source)
            src_tb.configure(state="disabled")

            trans_tb = ctk.CTkTextbox(
                card, height=90, corner_radius=6,
                border_width=1, border_color=("#2563EB", "#2563EB"),
                font=ctk.CTkFont(size=12),
            )
            trans_tb.grid(row=2, column=1, sticky="nsew", padx=(6, 12), pady=(0, 4))
            trans_tb.insert("1.0", failed or source)

            warn_text = "\n".join(f"\u26a0 {w}" for w in warning_list)
            warn_box = ctk.CTkFrame(card, corner_radius=6, fg_color=("#FEF2F2", "#2D1B1B"))
            warn_box.grid(row=3, column=0, columnspan=2, sticky="ew", padx=12, pady=(4, 4))
            warn_box.grid_columnconfigure(0, weight=1)
            ctk.CTkLabel(
                warn_box, text=warn_text, anchor="w", wraplength=700,
                text_color=("#DC2626", "#FCA5A5"), font=ctk.CTkFont(size=11),
                justify="left",
            ).grid(row=0, column=0, sticky="ew", padx=8, pady=6)

            status_lbl = ctk.CTkLabel(
                card, text="", anchor="w",
                text_color=("#656D76", "#8B949E"), font=ctk.CTkFont(size=11),
            )
            status_lbl.grid(row=4, column=0, columnspan=2, sticky="ew", padx=12, pady=(2, 4))

            btn_frame = ctk.CTkFrame(card, fg_color="transparent")
            btn_frame.grid(row=5, column=0, columnspan=2, sticky="ew", padx=12, pady=(0, 8))
            btn_frame.grid_columnconfigure(0, weight=1)

            ctk.CTkButton(
                btn_frame, text="保存", width=60, height=28, corner_radius=6,
                fg_color=("#2563EB", "#2563EB"), hover_color=("#1D4ED8", "#1D4ED8"),
                command=lambda k=key, tb=trans_tb, sl=status_lbl:
                    self._save_review_entry(k, tb, sl),
            ).grid(row=0, column=1, padx=(0, 6), sticky="e")

            retrans_btn = ctk.CTkButton(
                btn_frame, text="重新翻译", width=80, height=28, corner_radius=6,
                fg_color=("#F59E0B", "#B45309"), hover_color=("#D97706", "#92400E"),
            )
            retrans_btn.configure(
                command=lambda rb=retrans_btn, k=key, s=source, tb=trans_tb, sl=status_lbl:
                    self._retranslate_single(k, s, tb, sl, rb),
            )
            retrans_btn.grid(row=0, column=2, padx=(0, 6), sticky="e")

            ctk.CTkButton(
                btn_frame, text="忽略", width=60, height=28, corner_radius=6,
                fg_color=("#6B7280", "#4B5563"), hover_color=("#4B5563", "#374151"),
                command=lambda k=key, c=card: self._ignore_review_entry(k, c),
            ).grid(row=0, column=3, sticky="e")

            self._review_data[key] = {
                "source": source,
                "failed": failed,
                "textbox": trans_tb,
                "status_label": status_lbl,
                "retrans_btn": retrans_btn,
                "frame": card,
            }

    def _review_entries(self, report: TranslationReport) -> list[tuple[str, list[str]]]:
        entries: OrderedDict[str, list[str]] = OrderedDict()
        for key, warning_list in report.warnings.items():
            entries[key] = list(warning_list)
        for failed_entry in report.failed_entries:
            key, _, error = failed_entry.partition(":")
            key = key.strip()
            if not key:
                continue
            if key in entries:
                continue
            message = error.strip() or failed_entry
            entries.setdefault(key, []).append(f"API 调用失败：{message}")
        return list(entries.items())

    def _hide_review_panel(self) -> None:
        for widget in self._review_scroll.winfo_children():
            widget.destroy()
        self._review_data.clear()
        self.review_panel.grid_remove()
        self.review_panel.master.grid_rowconfigure(1, weight=0)

    def _save_review_entry(self, key: str, textbox: ctk.CTkTextbox, status_label: ctk.CTkLabel) -> None:
        report = self._review_report
        if report is None:
            status_label.configure(text="错误：无翻译报告。")
            return
        new_text = textbox.get("1.0", "end").strip()
        if not new_text:
            status_label.configure(text="错误：翻译内容为空。")
            return
        try:
            target = Path(report.target_file)
            if not target.exists():
                status_label.configure(text=f"错误：目标文件不存在 {target}")
                return
            if target.suffix == ".snbt":
                values = load_lang_snbt(target)
                values[key] = new_text
                write_lang_snbt(target, values)
            elif target.is_dir():
                self._save_chapter_review_entry(target, key, new_text)
            else:
                status_label.configure(text=f"错误：无法识别目标文件 {target}")
                return
            status_label.configure(text="已保存 \u2713")
        except Exception as exc:
            status_label.configure(text=f"保存失败：{exc}")

    def _save_chapter_review_entry(self, chapters_dir: Path, key: str, new_text: str) -> None:
        parts = key.split(":", 2)
        if len(parts) != 3:
            raise ValueError(f"章节映射 key 格式无效：{key}")
        filename, index_text, _segment_key = parts
        try:
            segment_index = int(index_text)
        except ValueError as exc:
            raise ValueError(f"章节映射序号无效：{key}") from exc
        chapter_path = chapters_dir / filename
        if not chapter_path.exists():
            raise FileNotFoundError(f"章节文件不存在：{chapter_path}")
        replaced = replace_chapter_segments(chapter_path, {segment_index: new_text})
        if replaced != 1:
            raise ValueError(f"未能定位章节文本段：{key}")

    def _retranslate_single(
        self, key: str, source: str,
        textbox: ctk.CTkTextbox, status_label: ctk.CTkLabel,
        button: ctk.CTkButton | None,
    ) -> None:
        api_key = self.api_key.get().strip()
        if not api_key:
            status_label.configure(text="错误：请先配置 API Key。")
            return
        if button:
            button.configure(state="disabled", text="翻译中...")

        def worker():
            try:
                selected_api_key, model, style, base_url = self._current_translator_settings(api_key)
                translator = DeepSeekTranslator(
                    api_key=selected_api_key,
                    model=model,
                    base_url=base_url,
                )
                protected_source, protections = protect_text(source)
                batch = OrderedDict([(key, protected_source)])
                result = translator.translate_batch(batch, style=style)
                translated = result.get(key, protected_source)
                restored = restore_text(translated, protections)
                warnings = preserved_token_warnings(source, restored)
                if warnings:
                    self.after(0, lambda t=restored: self._on_retranslate_result(
                        textbox, status_label, button, t,
                        f"仍有 {len(warnings)} 个告警，可手动编辑后保存。",
                    ))
                else:
                    self.after(0, lambda: self._on_retranslate_result(
                        textbox, status_label, button, restored, "翻译成功 \u2713",
                    ))
            except Exception as exc:
                self.after(0, lambda e=exc: self._on_retranslate_result(
                    textbox, status_label, button, "",
                    f"翻译失败：{e}",
                ))

        thread = threading.Thread(target=worker, daemon=True)
        thread.start()

    def _on_retranslate_result(
        self,
        textbox: ctk.CTkTextbox,
        status_label: ctk.CTkLabel,
        button: ctk.CTkButton | None,
        text: str,
        status: str,
    ) -> None:
        if text:
            textbox.delete("1.0", "end")
            textbox.insert("1.0", text)
        status_label.configure(text=status)
        if button:
            button.configure(state="normal", text="重新翻译")

    def _retranslate_all_review(self) -> None:
        if not self._review_data:
            return
        api_key = self.api_key.get().strip()
        if not api_key:
            self._review_subtitle.configure(text="错误：请先配置 API Key。")
            return
        self._retranslate_all_btn.configure(state="disabled", text="翻译中...")
        pending: list[tuple[str, str]] = []
        for key, data in self._review_data.items():
            pending.append((key, data["source"]))
        total = len(pending)
        self._review_subtitle.configure(text=f"正在逐条重新翻译 0/{total}。每条会独立处理，不会互相影响。")

        def worker():
            try:
                selected_api_key, model, style, base_url = self._current_translator_settings(api_key)
                translator = DeepSeekTranslator(
                    api_key=selected_api_key,
                    model=model,
                    base_url=base_url,
                )
            except Exception as exc:
                self.after(0, lambda e=exc: self._on_retranslate_all_error(e))
                return

            ok = 0
            warning_count = 0
            failed = 0
            for index, (key, source) in enumerate(pending, start=1):
                self.after(0, lambda k=key: self._set_review_entry_status(k, "正在重新翻译..."))
                try:
                    protected_source, protections = protect_text(source)
                    batch = OrderedDict([(key, protected_source)])
                    result = translator.translate_batch(batch, style=style)
                    translated = result.get(key, protected_source)
                    restored = restore_text(translated, protections)
                    warnings = preserved_token_warnings(source, restored)
                    if warnings:
                        warning_count += 1
                        self.after(0, lambda k=key, t=restored, w=warnings:
                            self._on_batch_retranslate(k, t, w))
                    else:
                        ok += 1
                        self.after(0, lambda k=key, r=restored:
                            self._on_batch_retranslate(k, r, []))
                except Exception as exc:
                    failed += 1
                    self.after(0, lambda k=key, e=exc: self._on_batch_retranslate_error(k, e))
                self.after(0, lambda i=index, o=ok, w=warning_count, f=failed:
                    self._update_retranslate_all_progress(i, total, o, w, f))
            self.after(0, lambda o=ok, w=warning_count, f=failed:
                self._on_retranslate_all_done(o, w, f))

        thread = threading.Thread(target=worker, daemon=True)
        thread.start()

    def _set_review_entry_status(self, key: str, text: str) -> None:
        data = self._review_data.get(key)
        if data is None:
            return
        label = data.get("status_label")
        if label:
            label.configure(text=text)

    def _on_batch_retranslate(self, key: str, text: str, warnings: list[str]) -> None:
        data = self._review_data.get(key)
        if data is None:
            return
        tb = data.get("textbox")
        sl = data.get("status_label")
        if tb:
            tb.delete("1.0", "end")
            tb.insert("1.0", text)
        if sl:
            if warnings:
                sl.configure(text=f"API 已返回，但仍有 {len(warnings)} 个格式告警，可手动编辑后保存。")
            else:
                sl.configure(text="重新翻译成功，确认无误后点击保存。")

    def _on_batch_retranslate_error(self, key: str, exc: Exception) -> None:
        self._set_review_entry_status(key, f"API 重试失败：{exc}")

    def _update_retranslate_all_progress(self, done: int, total: int, ok: int, warning_count: int, failed: int) -> None:
        self._review_subtitle.configure(
            text=f"正在逐条重新翻译 {done}/{total}。成功 {ok}，仍有格式告警 {warning_count}，API 失败 {failed}。"
        )

    def _on_retranslate_all_done(self, ok: int, warning_count: int, failed: int) -> None:
        self._retranslate_all_btn.configure(state="normal", text="全部重新翻译")
        self._review_subtitle.configure(
            text=f"重新翻译完成：成功 {ok} 条，仍需人工处理 {warning_count} 条，API 失败 {failed} 条。成功项仍需确认后点击保存。"
        )

    def _on_retranslate_all_error(self, exc: Exception) -> None:
        self._retranslate_all_btn.configure(state="normal", text="全部重新翻译")
        self._review_subtitle.configure(text=f"无法开始全部重新翻译：{exc}")

    def _ignore_review_entry(self, key: str, card: ctk.CTkFrame) -> None:
        card.grid_remove()
        self._review_data.pop(key, None)
        if not self._review_data:
            self._review_subtitle.configure(text="所有条目已处理。")
            self._review_badge.configure(text="待处理 0")
        else:
            self._review_badge.configure(text=f"待处理 {len(self._review_data)}")
            self._review_title.set(f"人工处理  —  {len(self._review_data)} 条需要确认的映射")
        self._refresh_review_layout()

    def _refresh_review_layout(self) -> None:
        for idx, data in enumerate(self._review_data.values()):
            frame = data.get("frame")
            if frame and frame.winfo_exists():
                frame.grid(row=idx, column=0, sticky="ew", pady=(0, 12))

    def _ignore_all_review(self) -> None:
        for widget in self._review_scroll.winfo_children():
            widget.destroy()
        self._review_data.clear()
        self._review_subtitle.configure(text="已忽略所有条目。")
        self._review_badge.configure(text="待处理 0")

    def _panel(
        self,
        parent: ctk.CTkFrame | ctk.CTkScrollableFrame,
        title: str,
        description: str,
    ) -> ctk.CTkFrame:
        panel = ctk.CTkFrame(
            parent,
            corner_radius=12,
            border_width=1,
            fg_color=("#FFFFFF", "#161B22"),
            border_color=("#D0D7DE", "#30363D"),
        )
        ctk.CTkLabel(
            panel,
            text=title,
            anchor="w",
            font=ctk.CTkFont(size=14, weight="bold"),
            text_color=("#1F2328", "#E6EDF3"),
        ).grid(
            row=0, column=0, columnspan=3, sticky="ew", padx=16, pady=(14, 2)
        )
        ctk.CTkLabel(
            panel, text=description, anchor="w",
            text_color=("#656D76", "#8B949E"),
            font=ctk.CTkFont(size=12),
        ).grid(
            row=1, column=0, columnspan=3, sticky="ew", padx=16, pady=(0, 10)
        )
        return panel

    def _show_view(self, view: str) -> None:
        self.workbench_frame.grid_remove()
        self.settings_frame.grid_remove()
        if view == "settings":
            self.settings_frame.grid()
        else:
            self.workbench_frame.grid()
            view = "workbench"
        self._set_nav_state(view)

    def _set_nav_state(self, active_view: str) -> None:
        for view, button in self._nav_buttons.items():
            if view == active_view:
                button.configure(fg_color="#2563EB", hover_color="#1D4ED8", text_color="#FFFFFF")
            else:
                button.configure(
                    fg_color=("#F3F4F6", "#21262D"),
                    hover_color=("#E5E7EB", "#30363D"),
                    text_color=("#374151", "#C9D1D9"),
                )

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

    def _current_translator_settings(self, fallback_api_key: str) -> tuple[str, str, str, str]:
        settings = self._run_settings
        if settings is not None:
            return (
                settings["api_key"] or fallback_api_key,
                settings["model"],
                settings["style"],
                settings["base_url"],
            )
        return (
            fallback_api_key,
            self.model.get() or DEFAULT_MODEL,
            self._current_style(),
            self.base_url.get() or DEFAULT_BASE_URL,
        )

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
            "idle": ("准备就绪", ("#DBEAFE", "#1E3A5F"), ("#1D4ED8", "#93C5FD")),
            "scanned": ("已完成扫描", ("#DCFCE7", "#14532D"), ("#16A34A", "#86EFAC")),
            "running": ("正在汉化", ("#FEF3C7", "#451A03"), ("#D97706", "#FCD34D")),
            "done": ("汉化完成", ("#DCFCE7", "#14532D"), ("#16A34A", "#86EFAC")),
            "error": ("需要处理", ("#FEE2E2", "#450A0A"), ("#DC2626", "#FCA5A5")),
        }
        text, bg_color, text_color = labels.get(stage, labels["idle"])
        self.stage.set(text)
        self.stage_badge.configure(fg_color=bg_color, text_color=text_color)

        active_order = ["idle", "scanned", "running", "done"]
        active_index = active_order.index(stage) if stage in active_order else 0
        for index, key in enumerate(active_order):
            label = self._step_labels.get(key)
            if label is None:
                continue
            if index <= active_index and stage != "error":
                label.configure(
                    fg_color=("#2563EB", "#2563EB"),
                    text_color="#FFFFFF",
                )
            else:
                label.configure(
                    fg_color=("#F3F4F6", "#1C2128"),
                    text_color=("#656D76", "#8B949E"),
                )

    def _choose_dir(self) -> None:
        directory = filedialog.askdirectory()
        if directory:
            self.selected_dir.set(directory)
            _log.info("User selected directory: %s", directory)
            self._scan()

    def _scan(self) -> None:
        self._review_report = None
        self._hide_review_panel()
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
        self._review_report = None
        self._hide_review_panel()
        self.progress.set(0)
        self.status.set("正在汉化...")
        self.progress_text.set("正在准备翻译任务...")
        self._set_stage("running")
        _log.info("Starting translate worker thread (quests_dir=%s, mode=%s)", self._quests_dir, mode)
        thread = threading.Thread(target=self._translate_worker, daemon=True)
        thread.start()

    def _translate_worker(self) -> None:
        assert self._quests_dir is not None
        settings = self._run_settings
        if settings is None:
            self._queue.put(("error", RuntimeError("翻译配置未初始化。")))
            return

        def progress(stage: str, done: int, total: int) -> None:
            self._queue.put(("progress", (stage, done, total)))

        def logger(message: str) -> None:
            self._queue.put(("log", message))

        try:
            _log.info("Calling translate_quests_auto with quests_dir=%s", self._quests_dir)
            report = translate_quests_auto(
                quests_dir=self._quests_dir,
                api_key=settings["api_key"],
                batch_size=settings["batch_size"],
                model=settings["model"],
                style=settings["style"],
                base_url=settings["base_url"],
                max_workers=settings["max_workers"],
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
                    stage, done, total = cast(_ProgressPayload, payload)
                    ratio = 1 if total == 0 else min(1, done / total)
                    self.progress.set(ratio)
                    self.status.set(f"{stage}: {done}/{total}")
                    self.progress_text.set(f"{stage}：{done}/{total}")
                elif kind == "done":
                    report = cast(TranslationReport, payload)
                    self.progress.set(1)
                    self.status.set("汉化完成。")
                    self.progress_text.set("汉化完成，可以查看日志和输出文件。")
                    self._set_stage("done")
                    self._log(f"完成：写入 {report.target_file}")
                    self._log(f"备份：{report.backup_dir}")
                    self._log(f"缓存命中：{report.cache_hits}，失败：{len(report.failed_entries)}")
                    self._review_report = report
                    if report.warnings or report.failed_entries:
                        self._log(f"告警：{len(report.warnings)} 条翻译存在格式 token 问题。")
                        if report.failed_entries:
                            self._log(f"失败映射：{len(report.failed_entries)} 条需要人工处理。")
                        self._show_review_entries()
                    else:
                        self._hide_review_panel()
                    self.scan_button.configure(state="normal")
                    self.translate_button.configure(state="normal")
                elif kind == "log":
                    self._log(str(payload))
                elif kind == "error":
                    exc = cast(Exception, payload)
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
