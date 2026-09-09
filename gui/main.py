"""
AnyKey - 任意键配置管理器（主入口）
基于 customtkinter 新拟态风格
"""

import tkinter as tk
from tkinter import ttk, messagebox, filedialog, font as tk_font
import customtkinter as ctk
import json
import os
import subprocess
import sys
import re
import webbrowser
import threading

# 确保作为独立进程启动时能找到上级模块（被托盘 subprocess 拉起时需要）
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

def _get_app_dir():
    """应用根目录：frozen 时 exe 在根目录，dev 时从 __file__ 推算"""
    if getattr(sys, 'frozen', False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# 主题设置（必须在 import 后、所有 widget 创建前）
ctk.set_appearance_mode("light")
ctk.set_default_color_theme("blue")

# 从子模块导入
from lib.config import (
    EMPTY_LEADER, EMPTY_LAYERS, CONFIG_FILE, _BUNDLE_DIR,
    load_config, save_config,
    _is_single_key, _layer_display, _layer_from_display, _norm_key,
    normalize_layers_key_outputs, normalize_key_output,
)
from gui.components import _THEME, _CapsuleScrollbar
from gui.dialogs import _RowDialog, _KeyMapDialog, _LayerDialog, LargeInputDialog
from lib.ipc import IpcClient, CMD_PAUSE, CMD_RESUME, CMD_RELOAD, CMD_STATUS
from app_bar import AppPanel, _set_theme, APP_NONE

# 卡片之间的统一间距（像素）。所有顶层卡片/页面区块之间的间隙都用这个值，保持一致。
CARD_GAP = 8

# 所有顶层卡片（顶栏/设备栏/按键速查/各页主卡）统一圆角，避免大小不一。
CARD_RADIUS = 12

# ──────────────────────────────────────────────
# 自定义控件：可单独控制下拉箭头（3点V形）颜色的 ComboBox
# customtkinter 5.2.2 的箭头颜色跟随 text_color（与文字共用），无独立参数；
# 这里重写 _draw，在父级绘制后强制把箭头刷成柔和灰，文字保持 text_color 不变。
# ──────────────────────────────────────────────
class _SoftArrowComboBox(ctk.CTkComboBox):
    def _draw(self, no_color_updates=False):
        super()._draw(no_color_updates)
        # 父级已按 text_color 给箭头上色；这里强制覆盖为白色，箭头（3点V形）在浅灰按钮上更清晰
        if self._state == tk.DISABLED:
            _arrow_col = self._text_color_disabled
        else:
            _arrow_col = "#ffffff"  # 白：箭头（3点V形）
        self._canvas.itemconfig("dropdown_arrow",
                                fill=self._apply_appearance_mode(_arrow_col))

# ──────────────────────────────────────────────
# 主窗口
# ──────────────────────────────────────────────
class AnyKeyApp(ctk.CTk):
    def __init__(self):
        # Windows 11 亮色主题标题栏色
        super().__init__(fg_color="#f3f3f3")
        # 设置DPI感知（必须在Tk创建之前）
        self._scaling = 1.0
        if sys.platform == "win32":
            try:
                import ctypes
                ctypes.windll.shcore.SetProcessDpiAwareness(1)
                hdc = ctypes.windll.user32.GetDC(0)
                dpi = ctypes.windll.gdi32.GetDeviceCaps(hdc, 88)
                ctypes.windll.user32.ReleaseDC(0, hdc)
                self._scaling = dpi / 96.0
            except:
                pass
        self.resizable(True, True)           # 启用原生窗口缩放边框
        self.title("AnyKey - 任意键 配置管理器")

        # 加载窗口图标（onefile模式下资源在sys._MEIPASS目录）
        icon_png = os.path.join(_BUNDLE_DIR, "icon.png")
        icon_ico = os.path.join(_BUNDLE_DIR, "icon.ico")
        if os.path.exists(icon_ico):
            try:
                self.wm_iconbitmap(icon_ico)
            except Exception as ex:
                print(f"wm_iconbitmap failed: {ex}")
        if os.path.exists(icon_png):
            try:
                from PIL import Image, ImageTk
                img = Image.open(icon_png).convert("RGBA")
                # 缩放为正方形
                sz = min(img.size)
                img = img.crop(((img.width-sz)//2, (img.height-sz)//2,
                                (img.width+sz)//2, (img.height+sz)//2))
                img = img.resize((64, 64), Image.LANCZOS)
                photo = ImageTk.PhotoImage(img)
                self.iconphoto(False, photo)
                self._icon_photo = photo  # 防止被 GC
            except Exception as ex:
                print(f"Icon photo failed: {ex}")
        else:
            print(f"Icon file not found: {icon_png}")

        # 先设置minsize再设置geometry，避免冲突
        self.minsize(960, 680)

        # 窗口居中
        self.update_idletasks()
        screen_w = self.winfo_screenwidth()
        screen_h = self.winfo_screenheight()
        init_w, init_h = 1020, 740
        x = (screen_w - init_w) // 2
        y = (screen_h - init_h) // 2
        self.geometry(f"{init_w}x{init_h}+{x}+{y}")

        self.cfg = load_config()
        # 迁移：旧 config 的序列缺 _rawSeq → 用 keys 拼接补上（保持显示一致）
        for seq in self.cfg.get("leader", {}).get("sequences", []):
            if not seq.get("_rawSeq"):
                seq["_rawSeq"] = "".join(seq.get("keys", []))
        # ── per-device 映射覆盖：权威配置 + 当前编辑目标 ──
        # _config_master：权威（全局映射段 + devices 覆盖桶）；self.cfg 的映射段=当前编辑目标的解析视图。
        # _edit_target：None=编辑全局；否则为设备 guid（或 VID:PID:type 回退键）。
        self._config_master = self._dc(self.cfg)
        self._edit_target = None
        self._pending_edit_target = self.cfg.get("lastEditTarget")  # 启动后恢复上一次设备状态
        self._config_master = self._dc(self.cfg)
        # tapDance 已是扁平格式 {base, fn1, fn2...}，无需转换
        self._global_entry_row = None
        self._global_entry_icon = None
        self._edit_banner = None
        self._device_rows = []
        # 提前初始化层相关属性（避免_load_data时访问未定义属性）
        self._layer_cards = []
        self._switch_key_vars = []
        self._layer_hold_vars = []
        self._layer_block_vars = []
        self._layer_key_trees = []
        self._layer_inner = None  # 将在_build_layer_tab中创建
        # 提前初始化 combo/序列表控件列表：避免 _build_ui 中 _select_edit_target
        # （→ _reload_editors → _populate_*_table）早于表格构建时访问未定义属性，
        # 触发 CTk __getattr__ 转发到 tkapp 报 "object has no attribute '_seq_row_widgets'"。
        self._combo_row_widgets = []
        self._seq_row_widgets = []
        self._seq_rows = []
        self._autofilled_switchkeys = set()  # 已自动填充的 (layer_name, phys_key)

        # ── 标签页 show/hide 缓存 ──────────────────
        self._tab_frames = {}      # tab_name -> CTkFrame（已构建的页面）
        self._tab_frames_built = set()  # 哪些已填充内容
        self._current_tab = None

        self._build_ui()
        self._load_data()

        # 恢复上次的编辑目标（全部设备/某设备）
        _last = self._pending_edit_target
        if _last is not None:
            _subs = self.cfg.get("subscribed_devices", [])
            _found = any(
                (d.get("guid") or f"{d.get('vid','')}:{d.get('pid','')}:{d.get('kind','keyboard')}") == _last
                for d in _subs
            )
            if _found:
                self._select_edit_target(_last)
        else:
            # lastEditTarget 为空 → 编辑全局，也需解析视图
            self._select_edit_target(None)
        self._pending_edit_target = None  # 用完清掉

        # 窗口居中
        self.update_idletasks()
        w, h = self.winfo_width(), self.winfo_height()
        x = (self.winfo_screenwidth() - w) // 2
        y = (self.winfo_screenheight() - h) // 2
        self.geometry(f"{w}x{h}+{x}+{y}")

        # ── IPC 客户端 + 托盘管理 ────────────────
        self._ipc = IpcClient(on_event=self._on_ipc_event)

        # 关闭窗口 → 直接退出 GUI（托盘独立运行）
        self.protocol("WM_DELETE_WINDOW", self.destroy)

        # 启动托盘进程（如果还没运行）
        self._ensure_tray_running()

        # 启动时延迟发 reload（后台线程，不阻塞 UI）
        self.after(1500, lambda: threading.Thread(
            target=lambda: self._ipc.send(CMD_RELOAD), daemon=True).start())

    def _ensure_tray_running(self):
        """如果托盘未运行，启动托盘进程（优先 Rust 版 anykey-tray.exe）"""
        if self._ipc.is_connected() or self._ipc.connect():
            self._refresh_status()
            return

        base_dir = _get_app_dir()

        # 候选托盘启动方式（按优先级）：
        #   1) 打包后 exe 在根目录
        #   2) 开发模式 Rust 编译产物
        #   3) 旧 Python 版（兼容回退）
        candidates = [
            (os.path.join(base_dir, "anykey-tray.exe"), []),
            (os.path.join(base_dir, "anykey-tray", "target", "release", "anykey-tray.exe"), []),
            (os.path.join(base_dir, "anykey-tray", "target", "debug", "anykey-tray.exe"), []),
            (os.path.join(base_dir, "tray", "main.py"), [sys.executable]),
        ]
        for tray_path, prefix in candidates:
            if not os.path.exists(tray_path):
                continue
            try:
                subprocess.Popen(
                    prefix + [tray_path],
                    cwd=base_dir,
                    creationflags=subprocess.DETACHED_PROCESS,
                )
            except Exception:
                continue
            self._ipc.wait_for_server(timeout=5.0)
            self._refresh_status()
            return

    def _on_ipc_event(self, event_name, payload):
        """IPC 事件回调（IPC 工作线程触发，用 after 回到主线程）"""
        print(f"[event] {event_name}: {payload}")
        if event_name == "tray_shutdown":
            self.after(0, self.destroy)
        elif event_name == "state_changed":
            self.after(0, self._update_run_pause_btn, payload)

    # ──────────────────────────────────────────────
    # 自动保存：各页面控件变更后立即持久化
    # ──────────────────────────────────────────────
    def _autosave(self, *_):
        """收集当前 UI 状态并立即写入配置文件"""
        try:
            cfg = self._collect_cfg()
            save_config(cfg)
            # self.cfg 保持为 _flush_active_mapping 写入的合并视图，
            # 不替换为 master 深拷贝（否则后续编辑会错误写入全局层）
            # 保存后覆盖/继承状态可能变化（例如刚改的继承条目变成覆盖），刷新颜色框
            try:
                self._apply_override_badges()
            except Exception:
                pass
            # 静默更新状态栏（如果可用）
            status = getattr(self, 'status_var', None)
            if status:
                status.set("✓ 已自动保存")
        except Exception as e:
            print(f"autosave error: {e}")

    def _schedule_autosave(self, event=None, delay_ms=500):
        """延迟触发自动保存（防抖：多次快速输入只保存最后一次）"""
        if hasattr(self, '_autosave_job') and self._autosave_job:
            try:
                self.after_cancel(self._autosave_job)
            except Exception:
                pass
        self._autosave_job = self.after(delay_ms, self._autosave)

    def _bind_autosave(self, widget):
        """给输入框/变量绑定自动保存触发"""
        if isinstance(widget, (ctk.CTkEntry, ctk.CTkTextbox)):
            widget.bind("<FocusOut>", self._schedule_autosave)
        elif isinstance(widget, ctk.StringVar):
            widget.trace_add("write", lambda *_: self._schedule_autosave())

    def _tooltip_on_hover(self, widget, text):
        """给 widget 绑定鼠标悬停提示"""
        tip_win = None
        def _enter(event):
            nonlocal tip_win
            if tip_win is not None:
                return
            tip_win = tk.Toplevel(widget)
            tip_win.wm_overrideredirect(True)
            tip_win.wm_geometry(f"+{event.x_root+12}+{event.y_root+10}")
            label = tk.Label(tip_win, text=text, font=("Microsoft YaHei", 9),
                             bg="#ffffe0", fg="#333333",
                             relief="solid", borderwidth=1,
                             padx=6, pady=2)
            label.pack()
        def _leave(event):
            nonlocal tip_win
            if tip_win is not None:
                tip_win.destroy()
                tip_win = None
        widget.bind("<Enter>", _enter)
        widget.bind("<Leave>", _leave)

    # ──────────────────────────────────────────────
    # 主 UI 框架
    # ──────────────────────────────────────────────
    # ── 操作按钮（点击前先完成当前编辑并保存） ───────
    def _run_with_flush(self, fn):
        """先触发焦点控件的编辑完成，再调用目标函数"""
        self.focus_set()  # 触发当前焦点控件的 FocusOut
        self._autosave()  # 立即保存
        fn()

    def _build_ui(self):
        self._tab_frames = {}
        self._current_tab = None
        top_bar = ctk.CTkFrame(self, fg_color=_THEME["card"], corner_radius=CARD_RADIUS, height=34)
        top_bar.pack(fill="x", padx=CARD_GAP, pady=(6, 0))
        top_bar.pack_propagate(False)
        for text, cmd in [("导入", lambda: self._run_with_flush(self._import_config)),
                          ("导出", lambda: self._run_with_flush(self._export_config))]:
            ctk.CTkButton(top_bar, text=text, command=cmd,
                          fg_color="transparent", text_color=_THEME["text_mid"],
                          font=("Microsoft YaHei", 12), width=56, height=26,
                          corner_radius=6, border_width=1,
                          border_color=_THEME["border"]).pack(side="left", padx=(12, 4), pady=4)

        self._tab_buttons = {}
        tab_frame = ctk.CTkFrame(top_bar, fg_color="transparent")
        tab_frame.pack(side="left", expand=True)
        def switch_tab(tab_name):
            for name, btn in self._tab_buttons.items():
                btn.configure(fg_color=_THEME["blue"] if name==tab_name else "transparent",
                              text_color="white" if name==tab_name else _THEME["text_dark"],
                              border_width=0 if name==tab_name else 1)
            # 先创建空容器（不在里面构建），pack 后再构建内容
            if tab_name not in self._tab_frames:
                self._tab_frames[tab_name] = ctk.CTkFrame(self._editor_area, fg_color="transparent")
            # 隐藏旧 tab，pack 新 tab
            if self._current_tab and self._current_tab in self._tab_frames:
                self._tab_frames[self._current_tab].pack_forget()
            self._tab_frames[tab_name].pack(fill="both", expand=True)
            self._current_tab = tab_name
            # 首次显示时填充内容（此时容器已就位，winfo_width 等均正确）
            if tab_name not in self._tab_frames_built:
                self._tab_frames[tab_name].update_idletasks()
                if tab_name == "combo": self._build_combo_tab(self._tab_frames[tab_name])
                elif tab_name == "sequences": self._build_sequences_tab(self._tab_frames[tab_name])
                elif tab_name == "layer": self._build_layer_tab(self._tab_frames[tab_name])
                self._tab_frames_built.add(tab_name)
        self._switch_tab = switch_tab

        for name, text in [("combo","Combo"), ("layer","Tapdance"), ("sequences","Leader")]:
            btn = ctk.CTkButton(tab_frame, text=text, command=lambda n=name: switch_tab(n),
                                fg_color="transparent", text_color=_THEME["text_dark"],
                                corner_radius=6, width=80, height=26,
                                font=("Microsoft YaHei",12,"bold"),
                                border_width=1, border_color=_THEME["border"])
            btn.pack(side="left", padx=2)
            self._tab_buttons[name] = btn

        # 帮助按钮
        self._help_btn = ctk.CTkButton(top_bar, text="?",
            command=self._on_help,
            fg_color=_THEME["card"], hover_color=_THEME["border"],
            text_color=_THEME["text_dark"], corner_radius=6, width=32, height=26,
            font=("Microsoft YaHei",12,"bold"))
        self._help_btn.pack(side="right", padx=(0, CARD_GAP), pady=4)

        self._run_pause_btn = ctk.CTkButton(top_bar, text="▶ 运行",
            command=lambda: self._run_with_flush(self._toggle_run_pause),
            fg_color=_THEME["orange"], hover_color=_THEME["orange_h"],
            text_color="white", corner_radius=6, width=76, height=26,
            font=("Microsoft YaHei",12,"bold"))
        self._run_pause_btn.pack(side="right", padx=(0, CARD_GAP), pady=4)

        main_body = ctk.CTkFrame(self, fg_color="transparent")
        main_body.pack(fill="both", expand=True, padx=CARD_GAP, pady=CARD_GAP)

        self._sidebar = ctk.CTkFrame(main_body, fg_color=_THEME["card"], corner_radius=CARD_RADIUS, width=210)
        self._sidebar.pack(side="left", fill="y", padx=(0, CARD_GAP), pady=0)
        self._sidebar.pack_propagate(False)
        self._current_app = ""  # v3: 当前编辑的应用（空=全局）
        self._build_app_panel()
        # 初始化应用下拉（config 中已有的 apps）
        self._update_edit_target_banner()
        # app 与 device 之间的分割线
        ctk.CTkFrame(self._sidebar, height=1, fg_color=_THEME["border"]
                     ).pack(fill="x", padx=8, pady=2)
        self._build_device_panel()

        right_col = ctk.CTkFrame(main_body, fg_color="transparent")
        right_col.pack(side="left", fill="both", expand=True)
        right_col.grid_rowconfigure(0, weight=1, minsize=200)
        right_col.grid_rowconfigure(1, weight=0, minsize=160)
        right_col.grid_columnconfigure(0, weight=1)

        self._editor_area = ctk.CTkFrame(right_col, fg_color="transparent")
        self._editor_area.grid(row=0, column=0, sticky="nsew", pady=0)

        # 当前编辑目标状态栏（banner 文字 + 当前页导入/导出/删除按钮）
        _banner_bar = ctk.CTkFrame(self._editor_area, fg_color=_THEME["card"],
                                   corner_radius=6, height=30)
        _banner_bar.pack(fill="x", padx=2, pady=(0, 4))
        _banner_bar.pack_propagate(False)
        self._edit_banner = ctk.CTkLabel(
            _banner_bar, text="全局基础层（所有设备默认继承）",
            font=("Microsoft YaHei", 11, "bold"), text_color=_THEME["text_light"],
            fg_color="transparent", anchor="w")
        self._edit_banner.pack(side="left", padx=(8, 4))

        # 右侧：当前页说明（纯文字）+ 导入/导出/删除按钮
        _page_btns = ctk.CTkFrame(_banner_bar, fg_color="transparent")
        _page_btns.pack(side="right", padx=(0, 6))
        ctk.CTkLabel(_page_btns, text="当前页", font=("Microsoft YaHei", 10),
                     text_color=_THEME["text_mid"], fg_color="transparent"
                     ).pack(side="left", padx=(0, 4))
        for text, cmd in [("导入", self._import_app_page),
                          ("导出", self._export_app_page),
                          ("删除", self._on_delete_page_clicked)]:
            ctk.CTkButton(_page_btns, text=text, command=cmd,
                          fg_color="transparent", hover_color=_THEME["border"],
                          text_color=_THEME["text_dark"], border_width=1,
                          border_color=_THEME["border"], corner_radius=5,
                          width=52, height=22, font=("Microsoft YaHei", 10)
                          ).pack(side="left", padx=(0, 2))

        self._inspector_area = ctk.CTkFrame(right_col, fg_color=_THEME["card"], corner_radius=CARD_RADIUS,
                                          border_width=1, border_color=_THEME["border"],
                                          height=160)
        self._inspector_area.grid(row=1, column=0, sticky="nsew", pady=(CARD_GAP, 0))
        self._inspector_area.pack_propagate(False)
        self._build_key_presets()

        switch_tab("combo")
        self._selected_kb_key = None
        self._last_focused_entry = None
        # 全局焦点跟踪：所有 Entry 获焦时记录
        self.bind_all("<FocusIn>", self._track_last_focus, add="+")

    def _prebuild_tab(self, tab_name):
        """后台预构建标签页内容（不 pack，首次切换秒开）"""
        if tab_name in self._tab_frames:
            return
        try:
            self._prebuilding = True  # 通知子函数跳过 UI 刷新
            f = ctk.CTkFrame(self._editor_area, fg_color="transparent")
            self._tab_frames[tab_name] = f
            if tab_name == "layer": self._build_layer_tab(f)
            elif tab_name == "sequences": self._build_sequences_tab(f)
        except Exception as e:
            print(f"prebuild {tab_name} error:", e)
            if tab_name in self._tab_frames:
                del self._tab_frames[tab_name]
        finally:
            self._prebuilding = False

    def _build_app_panel(self):
        """应用设置侧栏面板（设备栏上方）"""
        _set_theme(_THEME)
        self._app_panel = AppPanel(self._sidebar, width=192)
        self._app_panel.on_app_change = self._on_app_selected

    def _build_device_panel(self):
        inner = ctk.CTkFrame(self._sidebar, fg_color="transparent")
        inner.pack(fill="both", expand=True, padx=8, pady=4)
        ctk.CTkLabel(inner, text="设备设置", font=("Microsoft YaHei",14,"bold"),
                     text_color=_THEME["text_mid"]).pack(anchor="w")
        all_row = ctk.CTkFrame(inner, fg_color="transparent")
        all_row.pack(fill="x", pady=(2,2))
        ctk.CTkLabel(all_row, text="设备独立设置", font=("Microsoft YaHei",11),
                     text_color=_THEME["text_dark"]).pack(side="left")
        # perDevice 语义 = 「设备独立设置开关」：ON=启用独立设置（只有订阅设备走各自 map，
        # 未订阅透传）；OFF（默认）=只有全局设置生效，所有设备都经过全局映射。
        self._per_device_var = ctk.BooleanVar(value=self.cfg.get("perDevice", False))
        self._per_device_sw = ctk.CTkSwitch(all_row, text="", variable=self._per_device_var,
            command=self._toggle_per_device, width=36, switch_width=32, switch_height=16,
            progress_color=_THEME["green"])
        self._per_device_sw.pack(side="right")
        # Canvas + CapsuleScrollbar
        dev_container = ctk.CTkFrame(inner, fg_color=_THEME["card"],
                                     border_width=1, border_color=_THEME["border"],
                                     corner_radius=6)
        dev_container.pack(fill="both", expand=True, pady=(0,4))
        dev_container.grid_columnconfigure(0, weight=1)
        dev_container.grid_columnconfigure(1, weight=0)
        dev_container.grid_rowconfigure(0, weight=1)
        self._dev_canvas = tk.Canvas(dev_container, highlightthickness=0, bd=0,
                                      bg=_THEME["card"])
        dev_vsb = _CapsuleScrollbar(dev_container, orient="vertical",
                                     command=self._dev_canvas.yview,
                                     bg_color=_THEME["card"], slider_color="#888888",
                                     track_width=3, slider_thickness=8, auto_hide=True)
        self._dev_canvas.grid(row=0, column=0, sticky="nsew")
        dev_vsb.grid(row=0, column=1, sticky="ns")
        self._dev_canvas.configure(yscrollcommand=lambda *a: (dev_vsb.set(*a), dev_vsb._on_scroll()))
        self._dev_list = ctk.CTkFrame(self._dev_canvas, fg_color="transparent")
        dev_window = self._dev_canvas.create_window((0,0), window=self._dev_list, anchor="nw", tags="inner")
        self._dev_list.bind("<Configure>", lambda e: self._dev_canvas.configure(
            scrollregion=self._dev_canvas.bbox("all")))
        self._dev_canvas.bind("<Configure>", lambda e: self._dev_canvas.itemconfig("inner", width=e.width))
        btn_bar = ctk.CTkFrame(inner, fg_color="transparent")
        btn_bar.pack(fill="x", pady=(2,0))
        self._refresh_btn = ctk.CTkButton(btn_bar, text="刷新", command=self._refresh_devices,
                      fg_color=_THEME["blue"], hover_color=_THEME["blue_h"],
                      text_color="white", corner_radius=6, width=56, height=24,
                      font=("Microsoft YaHei",11))
        self._refresh_btn.pack(side="left", padx=(0,4))
        self._identify_btn = ctk.CTkButton(btn_bar, text="识别", command=self._toggle_identify,
            fg_color=_THEME["green"], hover_color=_THEME["green_h"],
            text_color="white", corner_radius=6, width=56, height=24,
            font=("Microsoft YaHei",11))
        self._identify_btn.pack(side="left")
        self._device_rows = []
        self._identifying = False
        self._refresh_devices()

    def _build_global_entry_row(self):
        """设备栏顶部『基础设置』伪条目：点击进入全局基础层编辑。"""
        # 默认未选中：白底（与设备行一致）、灰框、黑字；选中后只改边框颜色/粗细，
        # 不整体填充蓝色，避免文字被「蓝底蓝字」吞掉。
        gr = ctk.CTkFrame(self._dev_list, fg_color=_THEME["card"],
                          corner_radius=6, border_width=1,
                          border_color=_THEME["border"])
        # 与下方设备行一致：满宽、pady=2（让总高与设备卡 row_frame pack pady=1 + 内部 padding 接近）
        gr.pack(fill="x", pady=2)

        icon = ctk.CTkLabel(gr, text="🌐", font=("Microsoft YaHei", 24))
        icon.pack(side="left", padx=(4, 4), pady=(0, 4))
        txt = ctk.CTkFrame(gr, fg_color="transparent")
        txt.pack(side="left", padx=(6, 8), pady=(3, 3))
        t1 = ctk.CTkLabel(txt, text="基础设置", font=("Microsoft YaHei", 12, "bold"),
                          text_color=_THEME["text_dark"], anchor="w")
        t1.pack(fill="x", pady=(0, 0))
        t2 = ctk.CTkLabel(txt, text="所有设备的基础层", font=("Microsoft YaHei", 10),
                          text_color=_THEME["text_mid"], anchor="w")
        t2.pack(fill="x", pady=(0, 0))
        self._global_entry_row = gr
        self._global_entry_icon = icon

        # Hover：鼠标进入即提亮，离开恢复；选中态由蓝框承载，背景仍允许轻抬。
        def _on_enter(_e=None):
            try: gr.configure(fg_color=_THEME["card_hover"])
            except Exception: pass
        def _on_leave(_e=None):
            try: gr.configure(fg_color=_THEME["card"])
            except Exception: pass
        for _w in (gr, icon, txt, t1, t2):
            try:
                _w.bind("<Enter>", _on_enter)
                _w.bind("<Leave>", _on_leave)
                _w.bind("<Button-1>", lambda _e=None: self._select_edit_target(None))
            except Exception:
                pass

    def _refresh_devices(self):
        for w in self._dev_list.winfo_children():
            w.destroy()
        self._device_rows.clear()
        self._build_global_entry_row()
        from lib.driver import get_driver
        fd = get_driver()
        if fd.open():
            devices = fd.enum_devices()
            fd.close()
        else:
            devices = []

        # ── 读取 device_registry（fallback: subscribed_devices） ──
        registry = self.cfg.get("device_registry", None)
        if registry is None:
            # 迁移旧格式：用 subscribed_devices 初始化 registry
            registry = []
            for sd in self.cfg.get("subscribed_devices", []):
                entry = dict(sd)
                entry.setdefault("alias", "")
                entry.setdefault("online", True)
                registry.append(entry)
        reg_by_guid = {}
        reg_by_vp = {}
        for reg in registry:
            g = reg.get("guid")
            if g:
                reg_by_guid[g] = reg
            key = (str(reg.get("vid","")).lower(),
                   str(reg.get("pid","")).lower(),
                   (reg.get("type") or "keyboard").lower())
            reg_by_vp[key] = reg

        def _parse_vid_pid_from_hw(hw_id):
            import re as _re
            vid = pid = ""
            m = _re.search(r'(?i)VID_([0-9A-Fa-f]{4})', hw_id or "")
            if m: vid = m.group(1).upper()
            m = _re.search(r'(?i)PID_([0-9A-Fa-f]{4})', hw_id or "")
            if m: pid = m.group(1).upper()
            return vid, pid

        # ── 按 ContainerID 合并为一行显示 ──
        # 同一物理设备的键盘/鼠标/多媒体子节点共享一个 ContainerID（GUID）。
        # 显示合并成一行，但 _sub_devices 保留全部原始子节点，
        # _collect_cfg 据此为每个 hardware_id 写出独立的订阅条目。
        _grouped = {}
        _ungrouped = []
        for _d in devices:
            _g = (_d.get("container_id") or "").strip()
            if _g:
                _grouped.setdefault(_g, []).append(_d)
            else:
                _ungrouped.append(_d)

        def _is_generic_name(n):
            if not n:
                return True
            s = str(n).strip().lower()
            if ("hid keyboard" in s or "hid 键盘" in s
                    or "hid-compliant mouse" in s or "hid 兼容鼠标" in s
                    or "hid-compliant consumer" in s):
                return True
            return s in ("键盘设备", "鼠标", "hid keyboard device",
                         "hid-compliant mouse", "hid-compliant consumer control")

        def _merge_group(_devs, _g):
            any_kb = any(d.get("is_keyboard") for d in _devs)
            any_ms = any(d.get("is_mouse") for d in _devs)
            if any_kb and any_ms:
                type_label = "键盘 + 鼠标"
            elif any_kb:
                type_label = "键盘"
            elif any_ms:
                type_label = "鼠标"
            else:
                type_label = "其他"
            dtype = "keyboard" if any_kb else ("mouse" if any_ms else "other")
            # 挑一个真实产品名（避免泛用名 "HID Keyboard Device"）
            fname = _devs[0].get("hardware_id") or "未命名设备"
            for d in _devs:
                fn = d.get("friendly_name")
                if fn and not _is_generic_name(fn):
                    fname = fn
                    break
            if fname == (_devs[0].get("hardware_id") or "未命名设备"):
                for d in _devs:
                    if d.get("friendly_name"):
                        fname = d.get("friendly_name")
                        break
            m = dict(_devs[0])
            m["friendly_name"] = fname
            m["is_keyboard"] = any_kb
            m["is_mouse"] = any_ms
            m["container_id"] = _g
            m["_type_label"] = type_label
            m["_dtype"] = dtype
            m["_sub_devices"] = _devs  # 保存原始子节点供 _collect_cfg 写多条订阅
            return m, [d.get("device_id", 0) for d in _devs]

        _merged = []
        for _g, _devs in _grouped.items():
            _merged.append(_merge_group(_devs, _g))
        for _d in _ungrouped:
            _merged.append((_d, [_d.get("device_id", 0)]))

        # ── 创建 GUI 行（在线设备上，离线设备下） ──
        _matched_guids = set()

        def _add_device_row(dev, gids, dvid_s, dpid_s, dtype, type_label, guid,
                            enabled, alias_default, all_on, reg_entry, online=True):
            """为单个设备创建 GUI 行控件并注册到 _device_rows"""
            import customtkinter as ctk
            row_frame = ctk.CTkFrame(self._dev_list,
                                      fg_color=_THEME["card"],
                                      corner_radius=6,
                                      border_width=1,
                                      border_color=_THEME["border"])
            row_frame.pack(fill="x", pady=2)
            row_frame.grid_columnconfigure(0, minsize=38, weight=0)
            row_frame.grid_columnconfigure(1, weight=1)
            row_frame.grid_columnconfigure(2, minsize=22, weight=0)
            row_frame.grid_columnconfigure(3, minsize=58, weight=0)
            row_frame.grid_rowconfigure(0, weight=0)
            row_frame.grid_rowconfigure(1, weight=0)

            sw_disabled = (all_on or not online)
            sw_state = "disabled" if sw_disabled else "normal"
            sw_var = ctk.BooleanVar(value=enabled)
            sw = ctk.CTkSwitch(row_frame, text="", variable=sw_var, width=30,
                               switch_width=28, switch_height=14,
                               state=sw_state,
                               progress_color=(_THEME["text_disabled"] if sw_disabled else _THEME["blue"]),
                               button_color=(_THEME["text_disabled"] if sw_disabled else "white"),
                               command=lambda *_: self._schedule_autosave())
            sw.grid(row=0, column=0, padx=(4,2), pady=(3,3), sticky="w")

            alias_var = ctk.StringVar(value=alias_default)
            alias_var.trace_add("write", lambda *_: self._schedule_autosave())
            _entry_text_color = _THEME["text_disabled"] if all_on else (_THEME["text_dark"] if online else _THEME["text_disabled"])
            entry_state = "disabled" if (all_on or not online) else "normal"
            entry = ctk.CTkEntry(row_frame, textvariable=alias_var,
                                 font=("Microsoft YaHei",10,"bold"),
                                 text_color=_entry_text_color,
                                 fg_color=_THEME["card_bg"],
                                 border_width=1, corner_radius=6, height=24,
                                 state=entry_state)
            entry.grid(row=0, column=1, columnspan=3, padx=(2,4), pady=(4,1), sticky="ew")
            entry.bind("<FocusOut>", lambda e: self._schedule_autosave())

            _line2_text_color = _THEME["text_disabled"] if (all_on or not online) else "#000000"
            if dvid_s or dpid_s:
                info_text = f"{dvid_s}:{dpid_s}".upper()
            else:
                info_text = ""
            type_label_w = ctk.CTkLabel(row_frame, text=info_text,
                                        font=("Microsoft YaHei",9),
                                        text_color=_line2_text_color)
            type_label_w.grid(row=1, column=1, padx=(3,0), pady=(0,3), sticky="w")

            domain_label = ctk.CTkLabel(row_frame, text="域",
                                        font=("Microsoft YaHei",9),
                                        text_color=_line2_text_color)
            domain_label.grid(row=1, column=2, padx=(0,2), pady=(0,3), sticky="e")

            _domain_id = reg_entry.get("domain_id")
            domain_num = str(_domain_id) if isinstance(_domain_id, int) else "1"
            _preset_inv = {"0":"独立","1":"全局","2":"域2","3":"域3","4":"域4"}
            domain_display = _preset_inv.get(domain_num, domain_num)
            domain_var = ctk.StringVar(value=domain_display)
            domain_var.trace_add("write", lambda *_: self._schedule_autosave())
            domain_combo = _SoftArrowComboBox(row_frame,
                                           values=["独立","全局","域2","域3","域4"],
                                           variable=domain_var,
                                           width=56, height=24,
                                           font=("Microsoft YaHei",9),
                                           fg_color=_THEME["card_bg"],
                                           button_color=_THEME["border"],
                                           button_hover_color=_THEME["text_light"],
                                           dropdown_fg_color=_THEME["card_bg"],
                                           dropdown_hover_color=_THEME["card_hover"],
                                           dropdown_text_color=_THEME["text_dark"],
                                           text_color=_THEME["text_dark"],
                                           border_width=1,
                                           border_color=_THEME["border"],
                                           corner_radius=6,
                                           state="disabled" if all_on else "normal")
            domain_combo.grid(row=1, column=3, padx=(0,4), pady=(0,2), sticky="e")

            hint = ctk.CTkLabel(row_frame, text="", font=("Microsoft YaHei",9),
                                text_color=_THEME["green"])
            hint.grid(row=1, column=0, padx=(4,0), pady=(0,2), sticky="w")
            edit_key = self._device_edit_key(guid, dvid_s, dpid_s, dtype)
            # 点击设备行（避开开关/别名输入/域下拉）→ 进入该设备映射编辑
            def _on_row_click(_e=None, _k=edit_key):
                if _k:
                    self._select_edit_target(_k)
            for _w in (row_frame, type_label_w, hint, domain_label):
                try:
                    _w.bind("<Button-1>", _on_row_click)
                except Exception:
                    pass

            # 设备卡片 hover 反馈：debounce + 几何二次确认。
            # Enter 立即高亮、取消 pending 恢复；Leave 设 60ms 定时器，
            # 到时做几何检查：指针真移出卡片才恢复 card，仍在内则跳过。
            # 60ms 足够覆盖子控件间移动间隙，且恢复前有二次确认兜底，
            # 彻底避免"移出还亮着"和"子控件间闪烁"两个问题。
            _hover_timer = [None]
            def _on_dev_enter(_e=None):
                if _hover_timer[0] is not None:
                    try: row_frame.after_cancel(_hover_timer[0])
                    except Exception: pass
                    _hover_timer[0] = None
                try: row_frame.configure(fg_color=_THEME["card_hover"])
                except Exception: pass
            def _on_dev_leave(_e=None):
                if _hover_timer[0] is not None:
                    try: row_frame.after_cancel(_hover_timer[0])
                    except Exception: pass
                _hover_timer[0] = row_frame.after(60, _dev_restore_if_outside)
            def _dev_restore_if_outside():
                _hover_timer[0] = None
                try:
                    px = row_frame.winfo_pointerx() - row_frame.winfo_rootx()
                    py = row_frame.winfo_pointery() - row_frame.winfo_rooty()
                    w = row_frame.winfo_width()
                    h = row_frame.winfo_height()
                    if not (0 <= px <= w and 0 <= py <= h):
                        row_frame.configure(fg_color=_THEME["card"])
                except Exception:
                    pass
            for _w in [row_frame] + list(row_frame.winfo_children()):
                try:
                    _w.bind("<Enter>", _on_dev_enter)
                    _w.bind("<Leave>", _on_dev_leave)
                except Exception:
                    pass

            self._device_rows.append({
                "frame":row_frame, "switch":sw_var, "sw_widget":sw,
                "domain":domain_var, "domain_widget":domain_combo,
                "hint":hint, "device":dev,
                "vid":dvid_s, "pid":dpid_s, "guid":guid,
                "type":dtype, "alias_var":alias_var, "entry":entry,
                "device_ids":set(gids) if isinstance(gids, set) else gids,
                "type_label":type_label_w, "domain_label":domain_label,
                "online": online, "_edit_key": edit_key,
            })

        for dev, gids in _merged:
            hw_id = dev.get("hardware_id", "")
            hw_vid, hw_pid = _parse_vid_pid_from_hw(hw_id)
            dvid = dev.get("vendor_id", 0)
            dpid = dev.get("product_id", 0)
            dvid_s = (hw_vid or (f"{int(dvid):04X}" if dvid else "")).lower()
            dpid_s = (hw_pid or (f"{int(dpid):04X}" if dpid else "")).lower()
            dtype = dev.get("_dtype")
            type_label = dev.get("_type_label")
            if not type_label:
                # 兜底（未经由 _merge_group 的路径，如无 GUID 设备）
                if dev.get("is_mouse"):
                    dtype = "mouse"; type_label = "鼠标"
                elif dev.get("is_keyboard"):
                    dtype = "keyboard"; type_label = "键盘"
                else:
                    dtype = "other"; type_label = "多媒体"
            guid = dev.get("container_id", "")
            if guid:
                _matched_guids.add(guid)

            # 从 registry 查找匹配条目
            reg_entry = reg_by_guid.get(guid) or reg_by_vp.get((dvid_s, dpid_s, dtype), {})
            enabled = reg_entry.get("enabled", False)
            alias_default = reg_entry.get("alias", "")
            all_on = not self._per_device_var.get()
            if not alias_default:
                # 无别名时显示设备原始名
                name = dev.get("friendly_name", dev.get("hardware_id", "?"))
                alias_default = name[:22] if name else "?"

            _add_device_row(dev, gids, dvid_s, dpid_s, dtype, type_label,
                              guid, enabled, alias_default, all_on, reg_entry)

        # ── 离线别名设备（在 registry 中但未扫描到，且 alias 非空） ──
        offline_rows = []
        _buckets = (self._config_master.get("devices") or {}) if hasattr(self, "_config_master") else {}
        for reg in registry:
            guid = reg.get("guid","")
            if guid and guid in _matched_guids:
                continue  # 在线，已处理
            alias = reg.get("alias","").strip()
            # 覆盖桶保留：即使无别名，只要该设备有独立映射覆盖也保留离线行
            _bkey = self._device_edit_key(guid, reg.get("vid",""), reg.get("pid",""),
                                          reg.get("type","keyboard"))
            _has_bucket = bool(_bkey and _buckets.get(_bkey))
            if not alias and not _has_bucket:
                continue  # 无别名且无覆盖，不保留
            key = (str(reg.get("vid","")).lower(),
                   str(reg.get("pid","")).lower(),
                   (reg.get("type") or "keyboard").lower())
            if key in reg_by_vp and reg_by_vp[key].get("guid") in _matched_guids:
                continue  # 在线（无 GUID，但 VID/PID/type 匹配在线设备）
            # 用 registry 中的信息重建离线行
            offline_rows.append(reg)

        for reg in offline_rows:
            dtype = reg.get("type", "keyboard")
            _olabel = "离线(已命名)" if reg.get("alias","").strip() else "离线(有覆盖)"
            _add_device_row({}, set(), reg.get("vid",""), reg.get("pid",""),
                                 dtype, _olabel, reg.get("guid",""),
                                 reg.get("enabled", False), reg.get("alias",""),
                                 not self._per_device_var.get(), reg, online=False)
        # 递归滚轮绑定（设备行创建完后）
        _dev_wheel_fn = lambda e: self._dev_canvas.yview_scroll(-1 if e.delta>0 else 1, "units")
        def _bind_dev_children(w):
            for ch in w.winfo_children():
                ch.bind("<MouseWheel>", _dev_wheel_fn, add="+")
                _bind_dev_children(ch)
        _bind_dev_children(self._dev_list)
        self._dev_list.bind("<MouseWheel>", _dev_wheel_fn)
        # 高亮当前编辑目标（全局/某设备）
        try:
            self._update_device_selection_ui()
        except Exception:
            pass

    def _toggle_per_device(self):
        per_dev = self._per_device_var.get()
        _tc_disabled = _THEME["text_disabled"]
        _tc_light = _THEME["text_light"]
        _tc_dark = _THEME["text_dark"]
        # 未启用独立设置（全部走全局）时设备开关置灰/禁用；启用后恢复可交互
        _sw_track_on = _THEME["blue"] if per_dev else _THEME["text_disabled"]
        _sw_knob = "white" if per_dev else _THEME["text_disabled"]
        for row in self._device_rows:
            online = row.get("online", True)
            # 离线行始终禁用，不受 per_dev 影响
            sw = row.get("sw_widget")
            if sw:
                if not online:
                    sw.configure(state="disabled",
                                 progress_color=_THEME["text_disabled"],
                                 button_color=_THEME["text_disabled"])
                elif not per_dev:
                    sw.configure(state="disabled",
                                 progress_color=_sw_track_on,
                                 button_color=_sw_knob)
                else:
                    sw.configure(state="normal",
                                 progress_color=_THEME["blue"],
                                 button_color="white")
            # 未启用独立设置时保留各设备开关的内存状态（不强制改写），仅禁用交互。
            # 真实开关状态记录在 config，刷新/切换后保持一致，不会被改动。
            # 名称 Entry：禁用 + 文字变灰
            entry_w = row.get("entry")
            if entry_w:
                entry_w.configure(
                    text_color=_tc_disabled if not per_dev else _tc_dark,
                    state="disabled" if not per_dev else "normal")
            # 域下拉：禁用/恢复
            if "domain_widget" in row and row["domain_widget"] is not None:
                row["domain_widget"].configure(state="disabled" if not per_dev else "normal")
            # 类型标签 + 域标签：颜色切换（黑 ↔ 灰）
            _label_color = _tc_disabled if not per_dev else "#000000"
            for _label_key in ("type_label", "domain_label"):
                _lbl = row.get(_label_key)
                if _lbl:
                    _lbl.configure(text_color=_label_color)
            # 行背景和边框保持不变（不再改色）
        self._schedule_autosave()

    # step-4: 复合设备（键盘+鼠标共享 ContainerID）现在在
    # _refresh_devices 中按子节点独立成行，不再合并，故无需此合并函数。

    def _build_key_presets(self):
        """按键预设面板——两列布局，每列内按钮自动换行，超出滚动"""
        # 标题
        bar = ctk.CTkFrame(self._inspector_area, fg_color="transparent")
        bar.pack(fill="both", expand=True, padx=6, pady=4)
        title_row = ctk.CTkFrame(bar, fg_color="transparent")
        title_row.pack(fill="x", anchor="w")
        ctk.CTkLabel(title_row, text="按键速查", font=("Microsoft YaHei",10,"bold"),
                     text_color=_THEME["text_mid"]).pack(side="left")
        ctk.CTkLabel(title_row, text="点击按钮追加键名到当前焦点输入框",
                     font=("Microsoft YaHei",8), text_color=_THEME["text_light"]).pack(side="left", padx=6)

        # 滚动容器 (Canvas + CapsuleScrollbar)
        sc_container = ctk.CTkFrame(bar, fg_color="transparent", height=110)
        sc_container.pack(fill="both", expand=True, pady=(2, 0))
        sc_container.grid_columnconfigure(0, weight=1)
        sc_container.grid_columnconfigure(1, weight=0)
        sc_container.grid_rowconfigure(0, weight=1)
        preset_canvas = tk.Canvas(sc_container, highlightthickness=0, bd=0,
                                   bg=_THEME["card"])
        preset_vsb = _CapsuleScrollbar(sc_container, orient="vertical",
                                        command=preset_canvas.yview,
                                        bg_color=_THEME["card"], slider_color="#888888",
                                        track_width=3, slider_thickness=8, auto_hide=True)
        preset_canvas.grid(row=0, column=0, sticky="nsew")
        preset_vsb.grid(row=0, column=1, sticky="ns")
        preset_canvas.configure(yscrollcommand=lambda *a: (preset_vsb.set(*a), preset_vsb._on_scroll()))
        scroll = ctk.CTkFrame(preset_canvas, fg_color="transparent")
        pw = preset_canvas.create_window((0,0), window=scroll, anchor="nw", tags="inner")
        scroll.bind("<Configure>", lambda e: preset_canvas.configure(scrollregion=preset_canvas.bbox("all")))
        preset_canvas.bind("<Configure>", lambda e: preset_canvas.itemconfig("inner", width=e.width))

        # 两列布局（等宽）
        cols = ctk.CTkFrame(scroll, fg_color="transparent")
        cols.pack(fill="both", expand=True)
        cols.grid_columnconfigure(0, weight=1, uniform="col")
        cols.grid_columnconfigure(1, weight=1, uniform="col")
        left_col = ctk.CTkFrame(cols, fg_color="transparent")
        left_col.grid(row=0, column=0, sticky="nsew", padx=(0, 4))
        right_col = ctk.CTkFrame(cols, fg_color="transparent")
        right_col.grid(row=0, column=1, sticky="nsew", padx=(4, 0))

        # 尺寸参数：列宽 ~430px，按钮 40+1+1=42，每行最多 9 个
        BTN_W, BTN_H = 40, 18
        PER_ROW = 9
        FONT = ("Microsoft YaHei", 9)

        def render_category(parent, cat_name, btns):
            LBL_W = 42
            PER_ROW_WITH_LABEL = 6   # 带标题时按钮数（标题占 42px）
            PER_ROW_NEXT = 7        # 后续纯按钮行
            chunks = []
            for i in range(0, len(btns), PER_ROW_NEXT):
                chunks.append(btns[i:i + PER_ROW_NEXT])
            # 第一行：标题 + 前 6 按钮
            if chunks:
                row0 = ctk.CTkFrame(parent, fg_color="transparent")
                row0.pack(fill="x", pady=(4, 1))
                ctk.CTkLabel(row0, text=cat_name, font=("Microsoft YaHei", 9, "bold"),
                             text_color=_THEME["text_mid"], width=LBL_W, anchor="w").pack(side="left")
                first_chunk = chunks[0][:PER_ROW_WITH_LABEL]
                for label, value in first_chunk:
                    ctk.CTkButton(row0, text=label, width=BTN_W, height=BTN_H,
                        command=lambda v=value: (setattr(self, "_last_focused_entry", self.focus_get() if self.focus_get() and hasattr(self.focus_get(), "insert") else getattr(self, "_last_focused_entry", None)), self._commit_all_entries(), self._insert_preset(v)),
                        fg_color=_THEME["card_bg"], hover_color=_THEME["blue"],
                        text_color=_THEME["text_mid"], corner_radius=3,
                        font=FONT).pack(side="left", padx=1, pady=1)
                # 第一行剩下的按钮（如果有）也放同行，无标题
                rest = chunks[0][PER_ROW_WITH_LABEL:]
                for label, value in rest:
                    ctk.CTkButton(row0, text=label, width=BTN_W, height=BTN_H,
                        command=lambda v=value: (setattr(self, "_last_focused_entry", self.focus_get() if self.focus_get() and hasattr(self.focus_get(), "insert") else getattr(self, "_last_focused_entry", None)), self._commit_all_entries(), self._insert_preset(v)),
                        fg_color=_THEME["card_bg"], hover_color=_THEME["blue"],
                        text_color=_THEME["text_mid"], corner_radius=3,
                        font=FONT).pack(side="left", padx=1, pady=1)
            # 后续行：7 个纯按钮，开头缩进一个标签宽度
            for chunk in chunks[1:]:
                rowN = ctk.CTkFrame(parent, fg_color="transparent")
                rowN.pack(fill="x", pady=1)
                ctk.CTkLabel(rowN, text="", width=LBL_W).pack(side="left")
                for label, value in chunk:
                    ctk.CTkButton(rowN, text=label, width=BTN_W, height=BTN_H,
                        command=lambda v=value: (setattr(self, "_last_focused_entry", self.focus_get() if self.focus_get() and hasattr(self.focus_get(), "insert") else getattr(self, "_last_focused_entry", None)), self._commit_all_entries(), self._insert_preset(v)),
                        fg_color=_THEME["card_bg"], hover_color=_THEME["blue"],
                        text_color=_THEME["text_mid"], corner_radius=3,
                        font=FONT).pack(side="left", padx=1, pady=1)

        # 左列：修饰 / 导航 / 编辑 / 锁定 / 虚拟键(含程序)
        for cat_name, btns in [
            ("修饰", [("Ctrl+","{ctrl down}{ctrl up}"),("Alt+","{alt down}{alt up}"),
                       ("Shift+","{shift down}{shift up}"),("Win+","{lwin down}{lwin up}"),
                       ("LCtrl","{LCtrl}"),("RCtrl","{RCtrl}"),
                       ("LAlt","{LAlt}"),("RAlt","{RAlt}"),
                       ("LShift","{LShift}"),("RShift","{RShift}"),
                       ("LWin","{LWin}"),("RWin","{RWin}")]),
            ("导航", [("↑","{Up}"),("↓","{Down}"),("←","{Left}"),("→","{Right}"),
                       ("Home","{Home}"),("End","{End}"),("PgUp","{PgUp}"),("PgDn","{PgDn}"),
                       ("Ins","{Insert}"),("Del","{Delete}")]),
            ("编辑", [("Ent","{Enter}"),("Tab","{Tab}"),("Esc","{Escape}"),
                       ("Bksp","{Backspace}"),("Spc","{Space}"),
                       ("PrtSc","{PrintScreen}"),("Pause","{Pause}")]),
            ("锁定", [("Cap","{CapsLock}"),("Scr","{ScrollLock}"),("Num","{NumLock}")]),
            ("虚拟键", [("{Leader}","{leader}"),
                       ("fn1","{fn1}"),("fn2","{fn2}"),
                       ("bn1","{bn1}"),("bn2","{bn2}"),
                       ("tn0","{tn0}"),("tn1","{tn1}"),("tn2","{tn2}"),
                       ("sel1","{select 1}"),("sel2","{select 2}"),
                       ("运行","RUN:"),("停100ms","{Sleep 100}"),
                       ("停500ms","{Sleep 500}"),("停1s","{Sleep 1000}")]),
        ]:
            render_category(left_col, cat_name, btns)

        # 右列：Fn / 鼠标 / 媒体 / 小键盘
        for cat_name, btns in [
            ("Fn", [("F1","{F1}"),("F2","{F2}"),("F3","{F3}"),("F4","{F4}"),
                     ("F5","{F5}"),("F6","{F6}"),("F7","{F7}"),("F8","{F8}"),
                     ("F9","{F9}"),("F10","{F10}"),("F11","{F11}"),("F12","{F12}")]),
            ("鼠标按键", [("LBtn","{MouseLeft}"),("RBtn","{MouseRight}"),("MBtn","{MouseMiddle}"),
                           ("X1","{MouseSide1}"),("X2","{MouseSide2}"),
                           ("滚上","{WheelUp}"),("滚下","{WheelDown}"),
                           ("左移","MouseMove(-10,0)"),("右移","MouseMove(10,0)"),
                           ("上移","MouseMove(0,-10)"),("下移","MouseMove(0,10)")]),
            ("媒体", [("音量+","{Volume_Up}"),("音量-","{Volume_Down}"),("静音","{Volume_Mute}"),
                       ("播放","{Media_Play_Pause}"),("下一","{Media_Next}"),("上一","{Media_Prev}"),
                       ("停止","{Media_Stop}"),("后退","{Browser_Back}"),("前进","{Browser_Forward}")]),
            ("小键盘", [("N0","{Numpad0}"),("N1","{Numpad1}"),("N2","{Numpad2}"),("N3","{Numpad3}"),
                       ("N4","{Numpad4}"),("N5","{Numpad5}"),("N6","{Numpad6}"),("N7","{Numpad7}"),
                       ("N8","{Numpad8}"),("N9","{Numpad9}"),("N.","{NumpadDot}"),
                       ("N/","{NumpadDiv}"),("N*","{NumpadMult}"),("N-","{NumpadSub}"),
                       ("N+","{NumpadAdd}"),("NEnt","{NumpadEnter}"),("NLk","{NumLock}")]),
        ]:
            render_category(right_col, cat_name, btns)
        # 递归滚轮绑定（子控件创建完后）
        _ps_wheel_fn = lambda e: preset_canvas.yview_scroll(-1 if e.delta>0 else 1, "units")
        def _bind_ps_children(w):
            for ch in w.winfo_children():
                ch.bind("<MouseWheel>", _ps_wheel_fn, add="+")
                _bind_ps_children(ch)
        _bind_ps_children(scroll)
        scroll.bind("<MouseWheel>", _ps_wheel_fn)
        preset_canvas.bind("<MouseWheel>", _ps_wheel_fn)

    def _insert_preset_global(self, key_str):
        if key_str is None:
            return
        dlg_target = getattr(self, "_dlg_target", None)
        if dlg_target is not None:
            try:
                cursor = dlg_target.index("insert")
                dlg_target.insert(cursor, key_str)
                return
            except Exception:
                pass
        entry = getattr(self, "_active_entry", None)
        if entry is None:
            entry = getattr(self, "_last_focused_entry", None)
        if entry is None:
            try:
                w = self.focus_get()
                if w is not None and hasattr(w, "insert"):
                    entry = w
            except Exception:
                pass
        if entry is None:
            return
        cur = ""
        if getattr(entry, "_showing_hint", False):
            entry._showing_hint = False
        else:
            try:
                cur = entry.get()
            except Exception:
                try:
                    cur = entry.get("1.0", "end-1c")
                except Exception:
                    pass
        try:
            entry.configure(textvariable=None)
            entry.delete(0, "end")
            entry.insert(0, cur + key_str)
            entry.configure(text_color=_THEME["text_dark"], font=("Consolas", 12))
        except Exception:
            try:
                entry.delete("1.0", "end")
                entry.insert("1.0", cur + key_str)
            except Exception:
                pass
        if hasattr(entry, "_var") and entry._var is not None:
            try:
                entry._var.set(cur + key_str)
            except Exception:
                pass

    def _track_last_focus(self, event):
        """全局焦点跟踪——含 CTkEntry（内部 Canvas→上查父控件含 insert）。"""
        w = event.widget
        if hasattr(w, "insert") and hasattr(w, "delete"):
            self._last_focused_entry = w
        elif hasattr(w, "master") and hasattr(w.master, "insert") and hasattr(w.master, "delete"):
            self._last_focused_entry = w.master

    def _toggle_run_pause(self):
        """运行/暂停：后台发 IPC，不阻塞 UI"""
        if not self._ipc.is_connected():
            print("[btn] IPC not connected, trying to reconnect...")
            if not self._ipc.connect():
                print("[btn] Reconnect failed")
                return

        btn = self._run_pause_btn
        is_running = btn.cget("text") == "⏸ 暂停"
        cmd = CMD_PAUSE if is_running else CMD_RELOAD
        print(f"[btn] Sending {cmd} (is_running={is_running})")

        def _run():
            try:
                resp = self._ipc.send(cmd)
                print(f"[btn] Response: {resp}")
            except Exception as e:
                print(f"[btn] IPC error: {e}")
        threading.Thread(target=_run, daemon=True).start()

    def _refresh_status(self):
        """后台线程查询托盘状态，更新按钮"""
        def _query():
            resp = self._ipc.send(CMD_STATUS)
            self.after(0, self._update_run_pause_btn, resp)
        threading.Thread(target=_query, daemon=True).start()

    def _update_run_pause_btn(self, status=None):
        """更新运行/暂停按钮文字和颜色"""
        btn = getattr(self, '_run_pause_btn', None)
        if btn is None:
            return
        if status is None:
            return

        if status.get("running") and not status.get("paused"):
            btn.configure(text="⏸ 暂停", fg_color=_THEME["orange"], hover_color=_THEME["orange_h"])
        else:
            btn.configure(text="▶ 运行", fg_color=_THEME["green"], hover_color=_THEME["green_h"])

    def _on_help(self):
        """打开帮助文档"""
        help_path = os.path.join(_get_app_dir(), "assets", "help.md")
        if os.path.exists(help_path):
            webbrowser.open(help_path)
        else:
            print("帮助文档未找到")

    # ── Combo 标签页 ─────────────────────────
    def _build_combo_tab(self, parent):
        """Combo 配置标签页内容：全宽单栏"""
        parent.configure(fg_color="transparent")

        # 统一页面边距
        page = ctk.CTkFrame(parent, fg_color="transparent")
        page.pack(fill="both", expand=True, pady=0)

        # ── Combo 组合键（标题 + 窗口时间 + 流向控制）──────
        c1 = ctk.CTkFrame(page, corner_radius=CARD_RADIUS, fg_color=_THEME["card"], border_width=0)
        c1.pack(fill="both", expand=True, pady=0)

        hdr = ctk.CTkFrame(c1, fg_color="transparent")
        hdr.pack(fill="x", padx=14, pady=(12, 4))

        ctk.CTkLabel(hdr, text="Combo 组合键", font=("Microsoft YaHei", 14, "bold"),
                     text_color=_THEME["text_mid"]).pack(side="left")

        # ── 右侧：判定时间 + 流向控制 ──
        right_side = ctk.CTkFrame(hdr, fg_color="transparent")
        right_side.pack(side="right")

        # 窗口时间
        self.combo_time_var = ctk.StringVar(value=str(self.cfg.get("comboTime", 35)))
        self.combo_time_var.trace_add("write", lambda *_: self._schedule_autosave())
        time_container = ctk.CTkFrame(right_side, fg_color="transparent")
        time_container.pack(side="right")
        ctk.CTkLabel(time_container, text="判定时间", font=("Microsoft YaHei", 11),
                     text_color=_THEME["text_light"]).pack(side="left", padx=(0, 4))
        ctk.CTkEntry(time_container, textvariable=self.combo_time_var,
                     font=("Segoe UI", 13),
                     text_color=_THEME["text_dark"],
                     fg_color=_THEME["card"],
                     border_color=_THEME["border"], border_width=1,
                     corner_radius=6, justify="center",
                     width=52, height=28).pack(side="left")
        ctk.CTkLabel(time_container, text="ms", font=("Microsoft YaHei", 11),
                     text_color=_THEME["text_light"]).pack(side="left", padx=(2, 12))

        inner = ctk.CTkFrame(c1, fg_color="transparent")
        inner.pack(fill="both", expand=True, padx=12, pady=(4, 8))
        self._build_combo_editable_table(inner)

        # 用 cfg 数据填充 combo 表格
        self._populate_combo_table()

    # ──────────────────────────────────────────────
    # 可编辑 Combo 表格（列：层 | 按键1 | 按键2 | 输出 | 删除）
    # 末尾保留一个空白哨兵行等待输入
    # 表头与数据行在同一个 grid 内，保证列对齐
    # ──────────────────────────────────────────────
    def _build_combo_editable_table(self, parent):
        """构建可直接编辑的 Combo 表格，支持哨兵行和双击弹窗
        表头与数据行在同一个 grid 内，保证列对齐。"""
        self._combo_rows = []
        self._combo_sentinel_tids = []

        wrapper = ctk.CTkFrame(parent, fg_color="transparent")
        wrapper.pack(fill="both", expand=True, padx=4, pady=(0, 4))

        # ── 固定表头 ──
        hdr_frame = ctk.CTkFrame(wrapper, fg_color="transparent")
        hdr_frame.pack(fill="x", pady=(0, 0))

        canvas = tk.Canvas(wrapper, bg=_THEME["card"],
                           highlightthickness=0, relief="flat")
        vsb = _CapsuleScrollbar(wrapper, orient="vertical",
                                command=canvas.yview,
                                bg_color=_THEME["card"],
                                slider_color="#888888",
                                track_width=3,
                                slider_thickness=8)
        canvas.configure(yscrollcommand=lambda *a: (vsb.set(*a), vsb._on_scroll()))
        canvas.pack(side="left", fill="both", expand=True)
        vsb.pack(side="right", fill="y")

        self._combo_canvas = canvas

        inner = ctk.CTkFrame(canvas, fg_color="transparent")
        inner_win = canvas.create_window((0, 0), window=inner,
                                        anchor="nw", tags="inner")
        inner.bind("<Configure>", lambda e, c=canvas: (
            c.configure(scrollregion=c.bbox("all"))
        ))

        canvas.bind("<Configure>", lambda e, c=canvas: (
            c.itemconfig("inner", width=e.width - 2)
        ))
        self._combo_inner = inner
        self._combo_table_inner = inner

        def _on_wheel(e):
            canvas.yview_scroll(-1 if e.delta > 0 else 1, "units")
        canvas.bind("<MouseWheel>", _on_wheel)
        inner.bind("<MouseWheel>", _on_wheel)

        # 配置 5 列 — 表头（固定）和数据行（滚动）分别设置在各自的 frame 中
        col_weights = [1, 2, 2, 4, 0]
        col_minsize = [50, 80, 80, 160, 32]
        for ci, (w, m) in enumerate(zip(col_weights, col_minsize)):
            hdr_frame.grid_columnconfigure(ci, weight=w, minsize=m)
            inner.grid_columnconfigure(ci, weight=w, minsize=m)

        # 表头（固定不滚动）
        hdr_txts = ["层", "按键 1", "按键 2", "输出", ""]
        for ci, txt in enumerate(hdr_txts):
            lbl = ctk.CTkLabel(
                hdr_frame, text=txt,
                font=("Microsoft YaHei", 11, "bold"),
                text_color=_THEME["text_mid"]
            )
            lbl.grid(row=0, column=ci, sticky="ew", padx=4, pady=(0, 4))

        # 分隔线（固定不滚动）
        sep = ctk.CTkFrame(hdr_frame, height=1, fg_color=_THEME["border"])
        sep.grid(row=1, column=0, columnspan=5, sticky="ew", padx=4, pady=(0, 2))

        # 同步表头宽度
        def _sync_hdr_width(e):
            hdr_frame.configure(width=e.width)
            hdr_frame.pack_configure()
        canvas.bind("<Configure>", _sync_hdr_width, add="+")

        self._combo_next_grid_row = 0
        self._combo_row_widgets = []

    def _populate_combo_table(self):
        """用 self.cfg 数据填充 combo 可编辑表格"""
        # 清除旧行
        for rd in self._combo_row_widgets:
            for w in rd["widgets"]:
                try:
                    w.destroy()
                except Exception:
                    pass
        self._combo_rows.clear()
        self._combo_row_widgets.clear()
        self._combo_next_grid_row = 0  # 数据行从 row=0 开始

        for row in self.cfg.get("comboMap", []):
            self._append_combo_row(
                _layer_display(row.get("layer", "base")),
                row["key1"], row["key2"], row["output"]
            )
        # 末尾空白哨兵行
        self._append_combo_sentinel()

    def _append_combo_row(self, layer="0", key1="", key2="", output="",
                           is_sentinel=False):
        """追加一行到 combo 表格，返回创建的 var tuple"""
        parent = self._combo_table_inner
        row_idx = self._combo_next_grid_row
        self._combo_next_grid_row += 1

        layer_var  = ctk.StringVar(value=layer)
        key1_var   = ctk.StringVar(value=key1)
        key2_var   = ctk.StringVar(value=key2)
        output_var = ctk.StringVar(value=output)

        entry_kw = dict(
            fg_color=_THEME["card"],
            border_color=_THEME["border"],
            border_width=1, corner_radius=6, height=32,
            font=("Microsoft YaHei", 11)
        )

        e_layer  = ctk.CTkEntry(parent, textvariable=layer_var,
                                 width=50, justify="center", **entry_kw)
        e_key1   = ctk.CTkEntry(parent, textvariable=key1_var,  **entry_kw)
        e_key2   = ctk.CTkEntry(parent, textvariable=key2_var,  **entry_kw)
        e_output = ctk.CTkEntry(parent, textvariable=output_var, **entry_kw)

        e_layer.grid (row=row_idx, column=0, sticky="ew", padx=4, pady=4)
        e_key1.grid  (row=row_idx, column=1, sticky="ew", padx=4, pady=4)
        e_key2.grid  (row=row_idx, column=2, sticky="ew", padx=4, pady=4)
        e_output.grid(row=row_idx, column=3, sticky="ew", padx=4, pady=4)

        # 给每个输入框绑滚轮（CTkEntry 会吞掉滚轮事件）
        canvas = self._combo_canvas
        def _entry_wheel(e, c=canvas):
            c.yview_scroll(-1 if e.delta > 0 else 1, "units")
        for entry in [e_layer, e_key1, e_key2, e_output]:
            entry.bind("<MouseWheel>", _entry_wheel)

        # 双击弹出大窗口编辑
        for entry, var in [(e_layer, layer_var), (e_key1, key1_var),
                            (e_key2, key2_var), (e_output, output_var)]:
            entry.bind("<Double-Button-1>",
                        lambda e, v=var: self._popup_large_input(v))

        # 自动保存：失焦时触发
        for var in (layer_var, key1_var, key2_var, output_var):
            var.trace_add("write", lambda *_: self._schedule_autosave())

        # 删除按钮（哨兵行不显示，填写后变为普通行再加）
        del_btn = ctk.CTkButton(parent, text="✕",
                                 fg_color="transparent",
                                 hover_color=_THEME["red"],
                                 text_color=_THEME["text_light"],
                                 corner_radius=4,
                                 width=28, height=28,
                                 font=("Microsoft YaHei", 10, "bold"))
        del_btn.grid(row=row_idx, column=4, padx=4, pady=2)

        vars_tuple = (layer_var, key1_var, key2_var, output_var)
        row_data = {
            "vars": vars_tuple,
            "widgets": (e_layer, e_key1, e_key2, e_output, del_btn),
            "grid_row": row_idx,
            "is_sentinel": is_sentinel,
        }

        if is_sentinel:
            del_btn.configure(state="disabled", text="")
        else:
            idx_ref = [len(self._combo_rows)]  # 占位，后续用 row_data 定位
            del_btn.configure(
                command=lambda rd=row_data: self._del_combo_row(rd)
            )

        self._combo_rows.append(vars_tuple)
        self._combo_row_widgets.append(row_data)
        # 增量刷新：该行的覆盖/继承颜色框随输入实时更新；哨兵行会在函数内自行短路。
        for var in (layer_var, key1_var, key2_var, output_var):
            var.trace_add("write", lambda *_: self._apply_badge_to_combo_row(row_data))
        return vars_tuple

    def _append_combo_sentinel(self):
        """在末尾追加哨兵行，监听输入后自动变为普通行"""
        vars_tuple = self._append_combo_row(is_sentinel=True)
        layer_var, key1_var, key2_var, output_var = vars_tuple

        tids = []

        def _on_change(*_):
            # 只要任一字段有内容，就将哨兵升格为普通行，再追加新哨兵
            if any(v.get().strip() for v in (layer_var, key1_var, key2_var, output_var)):
                # 移除 trace
                for var, tid in zip((layer_var, key1_var, key2_var, output_var), tids):
                    try:
                        var.trace_remove("write", tid)
                    except Exception:
                        pass
                # 升格：找到对应 row_data，取消哨兵标记，激活删除按钮
                for rd in self._combo_row_widgets:
                    if rd["vars"] is vars_tuple and rd["is_sentinel"]:
                        rd["is_sentinel"] = False
                        del_btn = rd["widgets"][4]
                        del_btn.configure(state="normal", text="✕",
                                          command=lambda r=rd: self._del_combo_row(r))
                        break
                # 追加新哨兵
                self._append_combo_sentinel()

        for var in (layer_var, key1_var, key2_var, output_var):
            tid = var.trace_add("write", _on_change)
            tids.append(tid)

    def _del_combo_row(self, row_data):
        """删除一行 combo"""
        for w in row_data["widgets"]:
            if isinstance(w, tk.Widget):
                try:
                    w.destroy()
                except Exception:
                    pass
        if row_data in self._combo_row_widgets:
            self._combo_row_widgets.remove(row_data)
        if row_data["vars"] in self._combo_rows:
            self._combo_rows.remove(row_data["vars"])
        self._schedule_autosave()

    # ── 收集 combo 表格数据 ────────────────────────
    def _collect_combo_rows(self):
        """从 _combo_row_widgets 收集非空非哨兵的行"""
        result = []
        for rd in self._combo_row_widgets:
            if rd["is_sentinel"]:
                continue
            layer_v, key1_v, key2_v, output_v = rd["vars"]
            k1 = key1_v.get().strip()
            k2 = key2_v.get().strip()
            # combo 键名不应带 {}花括号（花括号仅在 output 中有意义）。
            # 用户通过速查面板点选时可能一并插入 {}，这里自动剥离。
            if k1.startswith("{") and k1.endswith("}"):
                k1 = k1[1:-1].strip()
            if k2.startswith("{") and k2.endswith("}"):
                k2 = k2[1:-1].strip()
            out = normalize_key_output(output_v.get())
            layer_raw = layer_v.get().strip()
            if k1 and k2 and out:
                result.append({
                    "key1": k1,
                    "key2": k2,
                    "output": out,
                    "layer": _layer_from_display(layer_raw) if layer_raw else "base",
                })
        return result

    # ── 保留的 combo_tree 兼容接口（供 _collect_cfg 使用）──
    @property
    def combo_tree(self):
        """向后兼容占位符，实际数据已由 _combo_rows 管理"""
        return None

    def _make_tree(self, parent, columns, headings, widths,
                   ondouble=None, height=None):
        # 包装容器，去掉所有边框
        wrapper = ctk.CTkFrame(parent, fg_color="transparent",
                               border_width=0)
        wrapper.pack(fill="both", expand=True, padx=12, pady=12)

        tree = ttk.Treeview(wrapper, columns=columns, show="headings",
                            selectmode="browse", height=height)
        # 直接在控件层清除焦点高亮框（ttk 不认 highlightthickness，需用底层 tk.call）
        tree.tk.call("ttk::style", "configure", "Treeview", "-relief", "flat")
        try:
            tree.tk.call(str(tree), "configure", "-highlightthickness", 0)
        except Exception:
            pass

        vsb = _CapsuleScrollbar(wrapper, orient="vertical",
                                command=tree.yview,
                                bg_color=_THEME["card"],
                                slider_color="#888888",
                                track_width=3,
                                slider_thickness=8)
        tree.configure(yscrollcommand=lambda *args: (vsb.set(*args), vsb._on_scroll()))

        # 无边框、纯文字风格
        style = ttk.Style()
        style.theme_use("clam")
        # 移除 clam 主题绘制的边框层（treearea.border 是那圈细线的来源）
        style.layout("Treeview", [
            ("Treeview.treearea", {"sticky": "nswe"})
        ])
        style.configure("Treeview",
            background=_THEME["card"], foreground=_THEME["text_dark"],
            fieldbackground=_THEME["card"], rowheight=46,
            font=("Segoe UI", 18), borderwidth=0,
            highlightthickness=0, rowseparator="")
        style.configure("Treeview.Heading",
            background="", foreground=_THEME["text_dark"],
            font=("Segoe UI", 17, "bold"), borderwidth=0,
            padding=(8, 4))
        style.map("Treeview",
                   background=[("selected", _THEME["select"])],
                   foreground=[("selected", _THEME["text_dark"])])

        # 四列平均分配
        total_w = sum(widths)
        for col, head, w in zip(columns, headings, widths):
            tree.heading(col, text=head, anchor="center")
            tree.column(col, width=total_w // len(columns),
                        minwidth=50, anchor="center")

        tree.pack(side="left", fill="both", expand=True)
        vsb.pack(side="right", fill="y")

        # 鼠标滚轮支持（在树形区域滚动时触发滚动）
        def _on_tree_mousewheel(e):
            tree.yview_scroll(-1 if e.delta > 0 else 1, "units")
        tree.bind("<MouseWheel>", _on_tree_mousewheel)
        # 递归绑定到所有子项
        def _bind_mousewheel_to_tree(widget):
            widget.bind("<MouseWheel>", _on_tree_mousewheel)
            for child in widget.winfo_children():
                _bind_mousewheel_to_tree(child)
        _bind_mousewheel_to_tree(tree)

        if ondouble:
            tree.bind("<Double-1>", lambda e, t=tree, fn=ondouble: (
                fn() if t.selection() else None
            ))
        return tree

    # ── 通用按钮行 ──
    def _row_btn(self, parent, add_cmd, edit_cmd, del_cmd):
        """在 parent 底部打包【添加 / 编辑 / 删除】按钮行"""
        ctk.CTkButton(parent, text="添加", command=add_cmd,
                      fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                      text_color="white", corner_radius=8,
                      width=88, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="left")
        ctk.CTkButton(parent, text="编辑", command=edit_cmd,
                      fg_color=_THEME["blue"], hover_color=_THEME["blue_h"],
                      text_color="white", corner_radius=8,
                      width=88, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)
        ctk.CTkButton(parent, text="删除", command=del_cmd,
                      fg_color=_THEME["red"], hover_color=_THEME["red_h"],
                      text_color="white", corner_radius=8,
                      width=88, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="left")

    # ── 层标签页（键盘图 + 编辑面板 + 宏列表）──────
    def _build_layer_tab(self, parent):
        """层配置：顶栏(标签+增删) + 键盘卡片(含控制) + 宏列表 + 编辑面板"""
        from gui.layout import (FULL_KEYBOARD, KB_WIDTH, KB_HEIGHT,
                                    MOUSE_X, MOUSE_Y, get_label)
        parent.configure(fg_color="transparent")

        # 预构建用自然尺寸（窗口未就位时无法知最终宽），pack 后 Configure 一次校正
        _pre = getattr(self, '_prebuilding', False)
        _s = 1.0  # 后续从 canvas 实际宽度重算

        cfg = self.cfg.get("tapDance", {})
        base_layer_map = cfg.get("base", {})
        # fn 层：按键名前缀 "fn" 获取所有功能层，按序号排序
        _fn_keys = sorted([k for k in cfg if k.startswith("fn")], key=lambda x: int(x[2:] or 0))
        if not _fn_keys:
            _fn_keys = ["fn1", "fn2"]

        self._layer_cards = []
        self._switch_key_vars = []
        self._layer_hold_vars = []
        self._layer_block_vars = []
        self._layer_key_trees = []
        self._syncing_layer_td = False
        self._current_layer_idx = 0

        # 索引 0 = base 层
        self._init_layer_data(0, {"name": "base", "keyMap": base_layer_map})
        # 索引 1+ = fn 层
        for i, fn_name in enumerate(_fn_keys):
            self._init_layer_data(i + 1, {"name": fn_name, "keyMap": cfg.get(fn_name, {})})

        page = ctk.CTkFrame(parent, fg_color="transparent")
        page.pack(fill="both", expand=True, pady=0)

        # ══ 统一卡片（默认时间 + 键盘图 + 宏列表 + 选中设置）══
        # 提前创建（使 edit_bar 等组件成为其真实子组件），但延后 pack 到 top_bar 之后
        unified_card = ctk.CTkFrame(page, fg_color=_THEME["card"], corner_radius=CARD_RADIUS)
        kb_card = unified_card

        # ══ 底部编辑条（先创建，后 pack 到统一卡片底部）══
        edit_bar = ctk.CTkFrame(unified_card, fg_color=_THEME["card"],
                                 corner_radius=6, height=40)
        # 暂不 pack，等统一卡片内容填充后 pack 到卡片内底部

        ctk.CTkLabel(edit_bar, text="选中:",
                     font=("Microsoft YaHei", 12),
                     text_color=_THEME["text_light"]).pack(side="left", padx=(8, 2))
        self._kb_key_label = ctk.CTkLabel(edit_bar, text="\u2014",
            font=("Consolas", 14, "bold"), text_color=_THEME["blue"],
            width=50, anchor="w")
        self._kb_key_label.pack(side="left", padx=(0, 4))
        # 移除"内部键名"标注（不再需要显示）

        # 7 个编辑字段，用间距分隔 tap/hold 和 dt/dh 组
        self._kb_tap_var = None
        self._kb_hold_var = None
        self._kb_ht_var = None
        self._kb_dt_var = None
        self._kb_dtt_var = None
        self._kb_dh_var = None
        self._kb_dht_var = None

        # 输入行
        entry_row = ctk.CTkFrame(edit_bar, fg_color="transparent")
        entry_row.pack(side="left", padx=(0, 0))

        # 7 个 entry，按顺序：tap / hold / hold ms / dbl tap / dbl ms / dbl hold / dbl ms
        hint_list = ["tap", "hold", "ms", "dbl tap", "ms", "dbl hold", "ms"]
        self._hint_labels = hint_list  # 弹窗显示字段名
        # 绑定的 StringVar 引用（None 表示未选中键）
        self._kb_vars = [None] * 7
        # entry 控件列表（顺序同上）
        self._kb_entries = []
        # 最近获焦的 entry（preset 按钮 / 切键时定位用）
        self._active_entry = None

        # 提示文字专属字体：倾斜 + 更灰
        HINT_FONT = ("Consolas", 12, "italic")
        HINT_COLOR = "#b8b8b8"

        def _attach_entry(width, hint):
            """创建 entry：手动管理内容（不绑 textvariable），灰色 hint / 黑色实值"""
            e = ctk.CTkEntry(entry_row,
                font=("Consolas", 12),
                fg_color=_THEME["card_bg"],
                border_color=_THEME["border"], border_width=1,
                corner_radius=4, height=26, width=width)
            e.pack(side="left", padx=(1, 1))
            e._hint = hint
            e._var = None          # 当前绑定的 StringVar（None = 未绑定）
            e._showing_hint = False
            e._modified = False    # 标记用户是否在本次聚焦中真正修改过内容
            # 初始显示 hint（倾斜 + 浅灰）
            e.configure(textvariable=None, text_color=HINT_COLOR, font=HINT_FONT)
            e.insert(0, hint)
            e._showing_hint = True

            def _on_focus_in(_ev, entry=e):
                # 进入聚焦只重置 _modified，不动文本/颜色。
                # 清 hint 必须在用户键入时做（_on_key），否则会变成"空 entry 被点
                # 一下再失焦 → hint 文本留在 entry 里被当作实值"。
                entry._modified = False
                # 记录最近获焦的 entry，供 preset 按钮定位
                self._active_entry = entry
                # 把 entry 的 focus 强制 set（让 _on_focus_in 在 dialog 抢焦时也能记上）
                try:
                    entry.focus_set()
                except Exception:
                    pass

            def _on_key(_ev, entry=e):
                # 用户开始输入才算"修改"
                if not entry._modified:
                    entry._modified = True
                    if entry._showing_hint:
                        # 首次输入时清掉 hint，进入正常黑色 + 正常字体
                        entry.delete(0, "end")
                        entry.configure(text_color=_THEME["text_dark"],
                                        font=("Consolas", 12))
                        entry._showing_hint = False

            def _on_focus_out(_ev, entry=e):
                # 仅在用户实际修改过时才写回 StringVar
                # 避免"点一下 → 失焦 → var.set('')"的覆盖
                if entry._modified and entry._var is not None:
                    val = entry.get()
                    try:
                        entry._var.set(val)
                    except Exception:
                        pass
                # 决定显示：进入 hint 分支用 _showing_hint（视觉态），不用 val
                if entry._showing_hint:
                    # 仍处于 hint 视觉态（用户没键入过，或刚刚清空后还是空）
                    entry.configure(textvariable=None,
                                    text_color=HINT_COLOR, font=HINT_FONT)
                    entry._modified = False
                    # 触发刷新（即使没改也可能要刷新宏列表状态）
                    self._schedule_autosave()
                    self._refresh_kb_labels()
                    self._refresh_macro_table()
                    return
                val_now = entry.get()
                if not val_now.strip():
                    # 用户清空了输入 → 回到 hint 态
                    entry.configure(textvariable=None,
                                    text_color=HINT_COLOR, font=HINT_FONT)
                    entry.delete(0, "end")
                    entry.insert(0, entry._hint)
                    entry._showing_hint = True
                else:
                    entry.configure(text_color=_THEME["text_dark"],
                                    font=("Consolas", 12))
                entry._modified = False
                # 触发刷新
                self._schedule_autosave()
                self._refresh_kb_labels()
                self._refresh_macro_table()

            e.bind("<FocusIn>", _on_focus_in)
            e.bind("<FocusOut>", _on_focus_out)
            e.bind("<Key>", _on_key, add="+")
            e.bind("<<Paste>>", _on_key, add="+")
            return e

        for i, (w, h) in enumerate(zip([80, 80, 65, 80, 65, 80, 65], hint_list)):
            ent = _attach_entry(w, h)
            self._kb_entries.append(ent)
            # 双击弹大输入窗口（便于长宏编辑）
            ent.bind("<Double-Button-1>", lambda ev, e=ent, idx=i: self._on_entry_double_click(ev, e, idx))
            if i in (2, 4):  # 在 hold ms 后、dbl ms 后加分隔
                ctk.CTkLabel(entry_row, text="|",
                    font=("Consolas", 10), text_color=_THEME["text_light"]
                    ).pack(side="left", padx=(1, 1))

        # 兼容旧名：保留 7 个 _kb_*_e 引用（其它代码可能用到）
        (self._kb_tap_e, self._kb_hold_e, self._kb_ht_e,
         self._kb_dt_e, self._kb_dtt_e, self._kb_dh_e, self._kb_dht_e) = self._kb_entries

        ctk.CTkButton(edit_bar, text="清除", command=self._clear_kb_key,
                       fg_color="transparent", hover_color=_THEME["red"],
                       text_color=_THEME["text_light"],
                       border_width=1, border_color=_THEME["border"],
                       corner_radius=4, height=26, width=48,
                       font=("Microsoft YaHei", 11)).pack(side="left", padx=(3, 4))

        # ══ 顶栏：层标签(左) + 增删按钮(右) — 独立栏，不在卡片内 ══
        top_bar = ctk.CTkFrame(page, fg_color="transparent")
        top_bar.pack(side="top", fill="x")

        # ══ 现在才 pack 统一卡片（在顶栏之后）══
        unified_card.pack(side="top", fill="both", expand=True)

        # ══ 左箭头（层标签溢出时显示）══
        self._layer_left_arrow = ctk.CTkButton(top_bar, text="◀", width=22, height=28,
            corner_radius=6, font=("Microsoft YaHei", 10),
            fg_color=_THEME["card"], hover_color=_THEME["card_hover"],
            text_color=_THEME["text_mid"], command=lambda: self._scroll_layer_tabs(-1))

        # ══ 层标签滚动视口 ══
        self._layer_scroll = ctk.CTkCanvas(top_bar, bg="#f3f3f3",
            highlightthickness=0, height=34)
        self._layer_scroll.pack(side="left", fill="x", expand=True)
        self._layer_scroll.xscrollincrement = 60
        self._layer_tab_frame = ctk.CTkFrame(self._layer_scroll, fg_color="transparent")
        self._layer_scroll.create_window((0, 3), window=self._layer_tab_frame, anchor="nw")
        self._layer_tab_ready = False
        self._layer_tab_frame.bind(
            "<Configure>",
            lambda e: (self._layer_scroll.configure(scrollregion=self._layer_scroll.bbox("all")),
                       self._update_layer_arrows(),
                       self._fit_layer_canvas_height()) if getattr(self, '_layer_tab_ready', True) else None)
        self._layer_scroll.bind("<Configure>",
            lambda e: self._update_layer_arrows() if getattr(self, '_layer_tab_ready', True) else None)
        # 鼠标滚轮横向滚动
        self._layer_scroll.bind("<MouseWheel>", self._on_layer_wheel)
        self._layer_tab_frame.bind("<MouseWheel>", self._on_layer_wheel)

        # ══ 右箭头 ══
        self._layer_right_arrow = ctk.CTkButton(top_bar, text="▶", width=22, height=28,
            corner_radius=6, font=("Microsoft YaHei", 10),
            fg_color=_THEME["card"], hover_color=_THEME["card_hover"],
            text_color=_THEME["text_mid"], command=lambda: self._scroll_layer_tabs(1))

        btn_frame = ctk.CTkFrame(top_bar, fg_color="transparent")
        btn_frame.pack(side="right")
        self._del_layer_btn = ctk.CTkButton(btn_frame, text="删除层",
                       command=self._del_layer,
                       fg_color=_THEME["red"], hover_color=_THEME["red_h"],
                       text_color="white", corner_radius=8,
                       width=60, height=28,
                       font=("Microsoft YaHei", 10))
        self._del_layer_btn.pack(side="left", padx=(0, 4))
        ctk.CTkButton(btn_frame, text="+ 添加层", command=self._add_layer,
                       fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                       text_color="white", corner_radius=8,
                       width=80, height=28,
                       font=("Microsoft YaHei", 10)).pack(side="left")

        # ── （流向控制已移除：layerToTapDance 不再需要）──

        # ── 卡片内：默认时间栏 ──
        timing_bar = ctk.CTkFrame(unified_card, fg_color="transparent", height=30)
        timing_bar.pack(side="top", fill="x", padx=10, pady=(8, 2))
        timing_bar.pack_propagate(False)
        ctk.CTkLabel(timing_bar, text="默认时间:",
            font=("Microsoft YaHei", 11), text_color=_THEME["text_light"]).pack(side="left", padx=(10, 4))
        self._td_ht_var = ctk.StringVar(value=str(self.cfg.get("tapDance", {}).get("holdTerm", 200)))
        ctk.CTkEntry(timing_bar, textvariable=self._td_ht_var,
            font=("Consolas", 11), width=60, height=24,
            placeholder_text="hold ms",
            fg_color=_THEME["card_bg"], border_color=_THEME["border"], border_width=1,
            corner_radius=4).pack(side="left", padx=(2, 6))
        self._td_dt_var = ctk.StringVar(value=str(self.cfg.get("tapDance", {}).get("doubleTapTerm", 250)))
        ctk.CTkEntry(timing_bar, textvariable=self._td_dt_var,
            font=("Consolas", 11), width=60, height=24,
            placeholder_text="dbl tap ms",
            fg_color=_THEME["card_bg"], border_color=_THEME["border"], border_width=1,
            corner_radius=4).pack(side="left", padx=(2, 6))
        self._td_dh_var = ctk.StringVar(value=str(self.cfg.get("tapDance", {}).get("doubleHoldTerm", 200)))
        ctk.CTkEntry(timing_bar, textvariable=self._td_dh_var,
            font=("Consolas", 11), width=60, height=24,
            placeholder_text="dbl hold ms",
            fg_color=_THEME["card_bg"], border_color=_THEME["border"], border_width=1,
            corner_radius=4).pack(side="left", padx=(2, 6))

        # ══ 键盘区域（统一卡片内）══

        # 层控件框架（只创建不 pack，由 _refresh_layer_controls 在需要时 pack）
        self._layer_ctrl_frame = ctk.CTkFrame(kb_card, fg_color="transparent")

        # 键盘画布
        kb_canvas_frame = ctk.CTkFrame(kb_card, fg_color="transparent")
        kb_canvas_frame.pack(fill="x", padx=10)

        # 先创建占位 canvas，pack 后拿到真实宽度再调整 _s 和尺寸
        self._layer_kb_canvas = tk.Canvas(kb_canvas_frame, bg=_THEME["card"],
                                           highlightthickness=0,
                                           height=KB_HEIGHT + 4,
                                           scrollregion=(0, 0, KB_WIDTH, KB_HEIGHT))
        self._layer_kb_canvas.pack(fill="x")
        self._layer_kb_canvas.update_idletasks()
        _cw = self._layer_kb_canvas.winfo_width() - 8
        if _cw > 200:
            _s = _cw / KB_WIDTH
        self._layer_kb_canvas.configure(
            height=int(KB_HEIGHT * _s) + 4,
            scrollregion=(0, 0, int(KB_WIDTH * _s), int(KB_HEIGHT * _s)))
        self._kb_scale = _s
        self._kb_ready = False

        def _apply_kb_resize():
            cw = self._layer_kb_canvas.winfo_width() - 8
            if cw < 200:
                return
            s = cw / KB_WIDTH
            if abs(s - self._kb_scale) > 0.02:
                self._layer_kb_canvas.scale("all", 0, 0, 1.0 / self._kb_scale, 1.0 / self._kb_scale)
                self._layer_kb_canvas.scale("all", 0, 0, s, s)
                self._kb_scale = s
                bh = int(KB_HEIGHT * s) + 4
                self._layer_kb_canvas.configure(height=bh,
                    scrollregion=(0, 0, KB_WIDTH * s, KB_HEIGHT * s))
                self._update_kb_fonts(s)

        def _on_kb_resize(e):
            _apply_kb_resize()

        self._layer_kb_canvas.bind("<Configure>", _on_kb_resize, add="+")

        # 绘制按键（坐标直接乘以 _s）
        self._layer_kb_rects = {}
        self._layer_kb_labels = {}
        self._layer_kb_slot_bgs = {}
        self._layer_kb_outputs = {}
        # 字号也按缩放创建
        self._kb_font_main_base = 12  # 未缩放基准值
        self._kb_font_sub_base  = 12
        self._kb_font_out_base  = 14
        self._kb_font_main = max(7, int(self._kb_font_main_base * _s))
        self._kb_font_sub  = max(7, int(self._kb_font_sub_base * _s))
        self._kb_font_out  = max(8, int(self._kb_font_out_base * _s))
        self._kb_font_main_font = tk_font.Font(family="Consolas", size=self._kb_font_main)
        self._kb_font_sub_font  = tk_font.Font(family="Consolas", size=self._kb_font_sub)
        self._kb_font_out_font  = tk_font.Font(family="Consolas", size=self._kb_font_out, weight="bold")
        self._kb_font_pause_font = tk_font.Font(family="Consolas", size=max(7, int(10 * _s)))

        for kid, pos in FULL_KEYBOARD.items():
            x, y, w, h = int(pos["x"]*_s), int(pos["y"]*_s), int(pos["w"]*_s), int(pos["h"]*_s)
            r = self._layer_kb_canvas.create_rectangle(
                x + 1, y + 1, x + w - 1, y + h - 1,
                fill="#D3D1C7", outline="#B4B2A9", width=1,
                tags=("key", kid))
            self._layer_kb_rects[kid] = r

            tw = max(1, w - 4)
            label = get_label(kid)
            if kid in ("Pause", "NumLock"):
                _main_f = self._kb_font_pause_font
                _sub_f  = self._kb_font_pause_font
            else:
                _main_f = self._kb_font_main_font
                _sub_f  = self._kb_font_sub_font
            if "\n" in label:
                top_txt, bot_txt = label.split("\n", 1)
                t1 = self._layer_kb_canvas.create_text(
                    x + w // 2, y + h // 2 - int(8*_s),
                    text=top_txt, font=_main_f,
                    width=tw, fill="#2C2C2A",
                    tags=("key", "key_label", kid))
                t2 = self._layer_kb_canvas.create_text(
                    x + w // 2, y + h // 2 + int(8*_s),
                    text=bot_txt, font=_sub_f,
                    width=tw, fill="#5F5E5A",
                    tags=("key", "key_label", kid))
                self._layer_kb_labels[kid] = (t1, t2)
            elif label:
                t = self._layer_kb_canvas.create_text(
                    x + w // 2, y + h // 2,
                    text=label, font=_main_f,
                    width=tw, fill="#2C2C2A",
                    tags=("key", "key_label", kid))
                self._layer_kb_labels[kid] = (t,)
            else:
                self._layer_kb_labels[kid] = ()

            # ── 4 个输出值槽位（tap TL / hold TR / dt BL / dh BR）──
            _slots = {}
            for _sk, _sx, _sy in [
                ("tap",  x + w * 0.35, y + h * 0.35),
                ("hold", x + w * 0.65, y + h * 0.35),
                ("dt",   x + w * 0.35, y + h * 0.65),
                ("dh",   x + w * 0.65, y + h * 0.65),
            ]:
                _st = self._layer_kb_canvas.create_text(
                    int(_sx), int(_sy),
                    text="", font=self._kb_font_out_font,
                    fill="#534AB7", anchor="center",
                    tags=("key", "key_output", kid),
                    state="hidden")
                _slots[_sk] = _st
            self._layer_kb_outputs[kid] = _slots

        # ── 鼠标按钮区（坐标也乘以 _s）──
        KW = int(44 * _s); KH = int(44 * _s); GAP = max(1, int(3 * _s))
        MX = int(MOUSE_X * _s)
        BOT = int(288 * _s)
        W2 = 2 * KW + GAP
        CX = MX + (W2 - KW) // 2

        title_y = BOT - 4 * (KH + GAP) - max(1, int(6 * _s))
        self._layer_kb_canvas.create_text(
            MX + W2 // 2, title_y,
            text="鼠标键", font=("Microsoft YaHei", max(8, int(10 * _s)), "bold"),
            fill=_THEME["text_light"], tags=("mouse",))

        mouse_keys = [
            ("MouseLeft",   "左键",   0, 3),
            ("MouseRight",  "右键",   1, 3),
            ("MouseMiddle", "中键",  -1, 2),
            ("MouseSide1",  "侧1",  0, 0),
            ("WheelUp",     "滚\u2191", 1, 1),
            ("MouseSide2",  "侧2",  0, 1),
            ("WheelDown",   "滚\u2193", 1, 0),
        ]
        for kid, label, col, row in mouse_keys:
            bx = CX if col == -1 else MX + col * (KW + GAP)
            by = BOT - (row + 1) * KH - row * GAP
            rid = self._layer_kb_canvas.create_rectangle(
                bx, by, bx + KW, by + KH,
                fill="#D3D1C7", outline="#B4B2A9", width=1,
                tags=("key", kid))
            self._layer_kb_rects[kid] = rid
            lt = self._layer_kb_canvas.create_text(
                bx + KW // 2, by + KH // 2,
                text=label, font=("Microsoft YaHei", 9),
                fill="#2C2C2A", tags=("key", "key_label", kid))
            self._layer_kb_labels[kid] = (lt,)
            # 鼠标按钮也需要 4 个输出槽位（tap/hold/dt/dh），与普通键一样分四角
            _slots = {}
            for _sk, (_sx_off, _sy_off) in [
                ("tap",  (0.35, 0.35)),
                ("hold", (0.65, 0.35)),
                ("dt",   (0.35, 0.65)),
                ("dh",   (0.65, 0.65)),
            ]:
                _st = self._layer_kb_canvas.create_text(
                    bx + int(KW * _sx_off), by + int(KH * _sy_off),
                    text="", font=("Consolas", 8, "bold"),
                    fill="#534AB7", tags=("key", "key_output", kid),
                    state="hidden")
                _slots[_sk] = _st
            self._layer_kb_outputs[kid] = _slots

        self._layer_kb_canvas.tag_bind("key", "<Button-1>",
            lambda e: self._on_kb_key_click(e))

        # 宏列表（键盘图下方，两列布局）
        ctk.CTkLabel(kb_card, text="宏列表", font=("Microsoft YaHei",13,"bold"), text_color=_THEME["text_mid"]).pack(anchor="w", padx=10, pady=(4,0))

        # ── 先 pack 编辑条到底部（预留空间），再让宏列表填剩余 ──
        edit_bar.pack(side="bottom", fill="x", padx=4, pady=(2, 4))
        edit_bar.pack_propagate(False)

        # 两列宏列表容器（共享滚动条）
        macro_frame = ctk.CTkFrame(kb_card, fg_color="transparent")
        macro_frame.pack(fill="both", expand=True, padx=10, pady=(0,6))
        macro_frame.grid_columnconfigure(0, weight=1)
        macro_frame.grid_columnconfigure(1, weight=1)
        macro_frame.grid_columnconfigure(2, weight=0)
        macro_frame.grid_rowconfigure(0, weight=1)

        _kw = tk_font.Font(family="Consolas", size=16).measure("X"*8)
        _FONT = ("Consolas", 16)

        # 左列
        left_col = ctk.CTkFrame(macro_frame, fg_color="transparent")
        left_col.grid(row=0, column=0, sticky="nsew", padx=(0, 2))
        self._macro_text = tk.Text(left_col, wrap="char", width=18, font=_FONT,
            bg=_THEME["card"], fg=_THEME["text_dark"], borderwidth=0,
            highlightthickness=0, padx=6, pady=4, state="disabled",
            cursor="arrow", takefocus=0, relief="flat")
        self._macro_text.configure(tabs=(_kw,))
        self._macro_text.tag_configure("val", lmargin2=_kw+12)
        self._macro_text.tag_configure("prefix", foreground="#1E5FBF", lmargin2=_kw+12)
        self._macro_text.tag_configure("hl", background="#AFA9EC", foreground="#000000", font=("Consolas",16,"bold"))
        left_col.grid_columnconfigure(0, weight=1)
        left_col.grid_rowconfigure(0, weight=1)
        self._macro_text.grid(row=0, column=0, sticky="nsew")

        # 右列
        right_col = ctk.CTkFrame(macro_frame, fg_color="transparent")
        right_col.grid(row=0, column=1, sticky="nsew", padx=(2, 0))
        self._macro_text_r = tk.Text(right_col, wrap="char", width=18, font=_FONT,
            bg=_THEME["card"], fg=_THEME["text_dark"], borderwidth=0,
            highlightthickness=0, padx=6, pady=4, state="disabled",
            cursor="arrow", takefocus=0, relief="flat")
        self._macro_text_r.configure(tabs=(_kw,))
        self._macro_text_r.tag_configure("val", lmargin2=_kw+12)
        self._macro_text_r.tag_configure("prefix", foreground="#1E5FBF", lmargin2=_kw+12)
        self._macro_text_r.tag_configure("hl", background="#AFA9EC", foreground="#000000", font=("Consolas",16,"bold"))
        right_col.grid_columnconfigure(0, weight=1)
        right_col.grid_rowconfigure(0, weight=1)
        self._macro_text_r.grid(row=0, column=0, sticky="nsew")

        # 共享滚动条（控制两列同步）
        vsb = _CapsuleScrollbar(macro_frame, orient="vertical",
            command=lambda *a: (self._macro_text.yview(*a), self._macro_text_r.yview(*a)),
            bg_color=_THEME["card"], slider_color="#888888", track_width=3,
            slider_thickness=8, auto_hide=True)
        vsb.grid(row=0, column=2, sticky="ns")

        def _on_yscroll(*_):
            # 取两列中较大的滚动范围决定滑块位置
            fr1 = self._macro_text.yview()
            fr2 = self._macro_text_r.yview()
            lo = min(fr1[0], fr2[0]); hi = max(fr1[1], fr2[1])
            vsb.set(lo, hi)
            vsb._on_scroll()
        self._macro_text.configure(yscrollcommand=_on_yscroll)
        self._macro_text_r.configure(yscrollcommand=_on_yscroll)

        self._macro_text.bind("<Button-1>", lambda e: self._on_macro_click(e, "left"))
        self._macro_text.bind("<MouseWheel>", lambda e: (self._macro_text.yview_scroll(-1 if e.delta>0 else 1, "units"), self._macro_text_r.yview_moveto(self._macro_text.yview()[0])))
        self._macro_text_r.bind("<Button-1>", lambda e: self._on_macro_click(e, "right"))
        self._macro_text_r.bind("<MouseWheel>", lambda e: (self._macro_text_r.yview_scroll(-1 if e.delta>0 else 1, "units"), self._macro_text.yview_moveto(self._macro_text_r.yview()[0])))

        self._selected_kb_key = None
        self._kb_vars = [None] * 7
        self._kb_tap_var = None
        self._kb_hold_var = None
        self._kb_ht_var = None
        self._kb_dt_var = None
        self._kb_dtt_var = None
        self._kb_dh_var = None
        self._kb_dht_var = None

        self._refresh_layer_tabs()
        self._refresh_layer_controls()
        self._refresh_kb_labels()
        self._refresh_macro_table()
        # 此时 f 已 pack，widget 尺寸正确
        self._layer_tab_ready = True
        self._fit_layer_canvas_height()
        self._kb_ready = True

    # ── 初始化层数据 ──
    def _init_layer_data(self, idx, layer_def):
        """idx=0 为 baseLayer，idx>=1 为 fn1, fn2..."""
        is_base = (idx == 0)

        if is_base:
            # baseLayer：无切换键、无长按、无阻挡
            switch_var = ctk.StringVar(value="")
            self._switch_key_vars.append(switch_var)
            hold_var = ctk.StringVar(value="")
            self._layer_hold_vars.append(hold_var)
            block_var = ctk.BooleanVar(value=False)
            self._layer_block_vars.append(block_var)
        else:
            # switchKeys 已退役：保留 dummy 元素对齐索引
            self._switch_key_vars.append(ctk.StringVar(value=""))
            hold_var = ctk.StringVar(value="")
            self._layer_hold_vars.append(hold_var)
            hold_var.trace_add("write", lambda *_: self._schedule_autosave())
            initial_block = layer_def.get("blockHold", False)
            block_var = ctk.BooleanVar(value=initial_block)
            self._layer_block_vars.append(block_var)
        key_map = layer_def.get("keyMap", {})
        vars_list = []
        for k, o in key_map.items():
            if isinstance(o, dict):
                vars_list.append((
                    ctk.StringVar(value=k),
                    ctk.StringVar(value=o.get("tap", "")),
                    ctk.StringVar(value=o.get("hold", "")),
                    ctk.StringVar(value=str(o.get("ht", ""))),
                    ctk.StringVar(value=o.get("dt", "")),
                    ctk.StringVar(value=str(o.get("dtt", ""))),
                    ctk.StringVar(value=o.get("dh", "")),
                    ctk.StringVar(value=str(o.get("dht", ""))),
                ))
            else:
                # 旧格式兼容：单个字符串当作 tap
                vars_list.append((
                    ctk.StringVar(value=k),
                    ctk.StringVar(value=o),
                    ctk.StringVar(value=""),
                    ctk.StringVar(value=""),
                    ctk.StringVar(value=""),
                    ctk.StringVar(value=""),
                    ctk.StringVar(value=""),
                    ctk.StringVar(value=""),
                ))
        # sentinel（空行）
        vars_list.append(tuple(ctk.StringVar(value="") for _ in range(8)))
        self._layer_key_trees.append({"name": layer_def.get("name"), "vars": vars_list})

    # ── 键盘点击（修复缩放后坐标偏移）──
    def _on_kb_key_click(self, event):
        # 先把 7 个 entry 中未提交的修改写回 StringVar（点画布不触发 FocusOut）
        for e in self._kb_entries:
            self._commit_entry(e)
        canvas = self._layer_kb_canvas
        items = canvas.find_overlapping(event.x, event.y, event.x, event.y)
        for item in items:
            tags = canvas.gettags(item)
            if "key" in tags:
                for tag in tags:
                    if tag not in ("key", "key_label", "key_output", "key_slot_bg") and not tag.startswith("legend"):
                        self._select_kb_key(tag)
                        return

    def _load_entry(self, entry, var):
        """切键时强制刷新 entry 显示。

        关键：不绑 textvariable，全部 insert/delete 手动管理。
        这样不论 ctk 的 tv 缓存如何，切键后 entry 里的文本一定与 var.get() 一致。
        """
        entry._var = var
        entry._modified = False
        # 先彻底清空
        try:
            entry.delete(0, "end")
        except Exception:
            pass
        if var is not None:
            val = var.get()
            if val:
                # 有值：黑色 + 正常字体
                entry.configure(textvariable=None,
                                text_color=_THEME["text_dark"],
                                font=("Consolas", 12))
                entry.insert(0, val)
                entry._showing_hint = False
                return
        # 空：灰色 hint + 倾斜
        HINT_FONT = ("Consolas", 12, "italic")
        entry.configure(textvariable=None,
                        text_color="#b8b8b8", font=HINT_FONT)
        entry.insert(0, entry._hint)
        entry._showing_hint = True

    def _commit_entry(self, entry):
        """把 entry 当前内容按 _modified 标志写回对应 StringVar，并清掉 _modified。

        切键时主动调用一次（替代等不到的 FocusOut）。
        """
        if not getattr(entry, "_modified", False):
            return
        val = entry.get()
        if entry._var is not None:
            try:
                entry._var.set(val)
            except Exception:
                pass
        entry._modified = False

    def _commit_all_entries(self):
        """把 7 个 entry 中所有 _modified=True 的写回 StringVar。"""
        if not hasattr(self, "_kb_entries"):
            return
        for e in self._kb_entries:
            self._commit_entry(e)
        # 提交后刷新当前选中键的覆盖/继承颜色框
        self._apply_badge_to_layer_key(getattr(self, "_selected_kb_key", None))

    def _select_kb_key(self, key_id):
        self._selected_kb_key = key_id
        norm = _norm_key(key_id)
        display = norm if norm != key_id.lower() else (key_id.upper() if len(key_id) == 1 else key_id)
        self._kb_key_label.configure(text=display)

        idx = self._current_layer_idx
        trees = self._layer_key_trees[idx]
        row = None
        for r in trees["vars"]:
            if len(r) >= 1 and r[0].get().strip().lower() == key_id.lower():
                row = r
                break
        if row is None:
            kv = ctk.StringVar(value=key_id)
            tv = ctk.StringVar(value="")
            hv = ctk.StringVar(value="")
            htv = ctk.StringVar(value="")
            dtv = ctk.StringVar(value="")
            dttv = ctk.StringVar(value="")
            dhv = ctk.StringVar(value="")
            dhtv = ctk.StringVar(value="")
            row = (kv, tv, hv, htv, dtv, dttv, dhv, dhtv)
            sentinel = trees["vars"][-1]
            if len(sentinel) >= 1 and sentinel[0].get() == "":
                trees["vars"].insert(-1, row)
            else:
                trees["vars"].append(row)
            def _on_change(*_):
                self._schedule_autosave()
                self._refresh_kb_labels()
                self._refresh_macro_table()
                self._apply_badge_to_layer_key(key_id)
            kv.trace_add("write", lambda *_: self._schedule_autosave())
            for v in (tv, hv, htv, dtv, dttv, dhv, dhtv):
                v.trace_add("write", _on_change)

        ev0 = row[1] if len(row) >= 2 else None
        ev1 = row[2] if len(row) >= 3 else None
        ev2 = row[3] if len(row) >= 4 else None
        ev3 = row[4] if len(row) >= 5 else None
        ev4 = row[5] if len(row) >= 6 else None
        ev5 = row[6] if len(row) >= 7 else None
        ev6 = row[7] if len(row) >= 8 else None
        # 兼容旧名
        self._kb_tap_var = ev0
        self._kb_hold_var = ev1
        self._kb_ht_var = ev2
        self._kb_dt_var = ev3
        self._kb_dtt_var = ev4
        self._kb_dh_var = ev5
        self._kb_dht_var = ev6
        # 新数据源
        self._kb_vars = [ev0, ev1, ev2, ev3, ev4, ev5, ev6]

        # 把 7 个 entry 切到新 StringVar（强制刷新显示）
        # 关键：先 commit 旧 entry（把 _modified 的内容写回旧 var），再切到新键。
        # 直接切键时原 entry 不会收到 FocusOut，未保存的修改会丢失。
        for entry in self._kb_entries:
            self._commit_entry(entry)
        for entry, var in zip(self._kb_entries, self._kb_vars):
            self._load_entry(entry, var)
        self._refresh_kb_colors()
        self._refresh_macro_table()
        self._schedule_autosave()
        self._apply_badge_to_layer_key(key_id)

        # 同步高亮宏列表对应行
        self._highlight_macro_key(key_id)

    # ── 编辑面板（紧凑两行）──
    def _insert_preset(self, key_str):
        """将预设按键插入到当前焦点输入框（编辑栏或弹窗）。"""
        # 1) 弹窗中的大输入框优先（如果还活着）
        dlg_target = getattr(self, "_dlg_target", None)
        if dlg_target is not None:
            try:
                # CTkTextbox：在当前光标处插入；配对修饰键把光标留在 {}{} 中间
                if "}{" in key_str:
                    mid = key_str.index("}{") + 1
                    dlg_target.insert("insert", key_str[:mid])
                    dlg_target.insert("insert", key_str[mid:])
                else:
                    dlg_target.insert("insert", key_str)
                dlg_target.focus_set()
                return
            except Exception:
                pass
        # 2) 定位最近获焦的可编辑控件（combo/leader 表格单元 / tapdance 编辑器 / 其它 entry）
        entry = self._resolve_preset_target()
        if entry is None:
            return
        # 3) 普通 entry（combo/leader 表格单元等，绑定 textvariable）：直接插入，textvariable 自动同步
        if not hasattr(entry, "_showing_hint"):
            try:
                cursor_pos = entry.index("insert")
            except Exception:
                cursor_pos = 0
            try:
                entry.insert(cursor_pos, key_str)
                # 光标定位：配对修饰键 → 放在 {}{} 中间，否则放在插入文本后面
                if "}{" in key_str:
                    new_cursor = cursor_pos + key_str.index("}{") + 1
                else:
                    new_cursor = cursor_pos + len(key_str)
                entry.icursor(new_cursor)
            except Exception:
                pass
            self._schedule_autosave()
            return
        # 4) tapdance 编辑器 entry（带 hint 逻辑，手动管理文本）
        if getattr(entry, "_showing_hint", False):
            cur = ""
            cursor_pos = 0
        else:
            cur = entry.get()
            try:
                cursor_pos = entry.index("insert")
            except Exception:
                cursor_pos = len(cur)
        new_val = cur[:cursor_pos] + key_str + cur[cursor_pos:]
        entry.configure(textvariable=None)
        entry.delete(0, "end")
        entry.insert(0, new_val)
        # 光标定位：配对修饰键 → 放在 {}{} 中间，否则放在插入文本后面
        if "}{" in key_str:
            new_cursor = cursor_pos + key_str.index("}{") + 1
        else:
            new_cursor = cursor_pos + len(key_str)
        try:
            entry.icursor(new_cursor)
        except Exception:
            pass
        try:
            entry.configure(text_color=_THEME["text_dark"], font=("Consolas", 12))
        except tk.TclError:
            entry.configure(fg=_THEME["text_dark"])
        entry._showing_hint = False
        entry._modified = True
        if hasattr(entry, "_var") and entry._var is not None:
            try:
                entry._var.set(new_val)
            except Exception:
                pass
        self._schedule_autosave()
        if hasattr(self, "_refresh_kb_labels"):
            self._refresh_kb_labels()
        if hasattr(self, "_refresh_macro_table"):
            self._refresh_macro_table()

    @staticmethod
    def _unwrap_entry(w):
        """把焦点 widget 归一化为最外层可编辑控件（CTkEntry/CTkTextbox 包装器优先）。

        CTk 控件的内部原生 Entry/Text 的 master 才是包装器；直接操作内部原生控件
        会丢失 textvariable / hint 等包装器逻辑，这里统一上查到包装器。
        """
        if w is None:
            return None
        if not (hasattr(w, "insert") and hasattr(w, "delete")):
            return None
        m = getattr(w, "master", None)
        if m is not None and hasattr(m, "insert") and hasattr(m, "delete") \
                and isinstance(m, (ctk.CTkEntry, ctk.CTkTextbox)):
            return m
        return w

    def _resolve_preset_target(self):
        """返回 preset 按钮应插入的最近获焦可编辑控件（优先当前可见的全局焦点跟踪）。"""
        for src in (getattr(self, "_last_focused_entry", None),
                    getattr(self, "_active_entry", None)):
            entry = self._unwrap_entry(src)
            if entry is None:
                continue
            # 只认当前仍可见的控件，避免插入到已被 tab 切换隐藏的旧格子
            try:
                if entry.winfo_viewable():
                    return entry
            except Exception:
                pass
        # 兜底：当前焦点
        try:
            w = self.focus_get()
        except Exception:
            w = None
        entry = self._unwrap_entry(w)
        if entry is not None:
            try:
                if entry.winfo_viewable():
                    return entry
            except Exception:
                pass
        # 最后退回 tap 框（层页默认）
        return getattr(self, "_kb_tap_e", None)

    def _clear_kb_key(self):
        if not self._selected_kb_key:
            return
        idx = self._current_layer_idx
        trees = self._layer_key_trees[idx]
        for r in trees["vars"]:
            if len(r) >= 1 and r[0].get().strip().lower() == self._selected_kb_key.lower():
                for v in r:
                    v.set("")
                break
        # 解绑所有 entry
        self._kb_vars = [None] * 7
        self._kb_tap_var = self._kb_hold_var = self._kb_ht_var = None
        self._kb_dt_var = self._kb_dtt_var = self._kb_dh_var = self._kb_dht_var = None
        for e in self._kb_entries:
            self._load_entry(e, None)
        self._kb_key_label.configure(text="\u2014")
        self._selected_kb_key = None
        self._refresh_kb_labels()
        self._refresh_kb_colors()
        self._refresh_macro_table()
        self._schedule_autosave()

    # ── 输出值格式化 ──
    @staticmethod
    def _fmt_output(val):
        """格式化输出值显示：{X}取键名缩写，RUN→R，单字符直显，其他→M"""
        _abbr = {
            "Backspace": "Bs", "CapsLock": "Cps", "Enter": "Ent",
            "Escape": "Esc", "Space": "Spc", "Tab": "Tab",
            "LShift": "Sft", "RShift": "Sft", "Shift": "Sft",
            "LCtrl": "Ctr",  "RCtrl": "Ctr",  "Ctrl": "Ctr",
            "LAlt": "Alt",   "RAlt": "Alt",   "Alt": "Alt",
            "LWin": "Win",   "RWin": "Win",   "Win": "Win",
            "AppsKey": "Mnu",
            "PrintScreen": "PrS", "ScrollLock": "ScL",
            "Insert": "Ins", "Delete": "Del",
            "PgUp": "Pu", "PgDn": "Pd",
            "PageUp": "Pu", "PageDown": "Pd",
            "Home": "Hom", "End": "End",
            "Up": "\u2191", "Down": "\u2193",
            "Left": "\u2190", "Right": "\u2192",
            "NumLock": "NmL",
            # 媒体键
            "Media_Play_Pause": "\u64ad\u653e",   # 播放
            "Media_Prev":      "\u4e0a\u9996",   # 上首
            "Media_Next":      "\u4e0b\u9996",   # 下首
            "Media_Stop":      "\u505c\u6b62",   # 停止
            "Volume_Mute":     "\u9759\u97f3",   # 静音
            "Volume_Up":       "\u97f3\u91cf+",  # 音量+
            "Volume_Down":     "\u97f3\u91cf-",  # 音量-
            # 鼠标侧键
            "MouseSide1": "\u4fa71",  "mouseside1": "\u4fa71",
            "MouseSide2": "\u4fa72",  "mouseside2": "\u4fa72",
        }
        if not val:
            return ""
        if val.startswith("RUN:"):
            return ("R", "#CC3333")
        # 先数 {} 组数：多个 {X}{Y}... = 宏（不是单键）
        import re
        brace_groups = re.findall(r'\{[^}]+\}', val)
        if len(brace_groups) > 1:
            return ("M", "#3366CC")
        if val.startswith("{") and val.endswith("}"):
            inner = val[1:-1]
            if inner.startswith("fn") or inner.startswith("bn"):
                return (inner, "#534AB7")
            # 大小写不敏感的缩写匹配：先原字 → 首字母大写 → 全小写
            _abbr_lower = {k.lower(): v for k, v in _abbr.items()}
            disp = _abbr_lower.get(inner.lower()) or inner
            return (disp, "#534AB7")
        if len(val) == 1:
            return (val, "#534AB7")
        return ("M", "#3366CC")

    # ── 键面槽位布局计算 ──
    def _calc_slot_layout(self, x, y, w, h, active_slots):
        """根据活跃的槽位列表计算每个槽位的中心位置、可用面积和背景区域。
        active_slots: ["tap"] / ["tap","hold"] / 等
        返回 {slot_key: (cx, cy, max_w, max_h, bg_coords)}
        bg_coords 为 (x1,y1,x2,y2) 矩形 或 (x1,y1,x2,y2,x3,y3) 多边形
        """
        n = len(active_slots)
        layout = {}
        corners = {"tap": (0,0), "hold": (1,0), "dt": (0,1), "dh": (1,1)}
        if n == 1:
            sk = active_slots[0]
            layout[sk] = (x + w*0.5, y + h*0.5, w*0.85, h*0.85, x+2, y+2, x+w-2, y+h-2)

        elif n == 2:
            s1, s2 = active_slots
            c1, c2 = corners[s1], corners[s2]
            if c1[1] == c2[1]:  # 同行 → 左右分
                # 短边=水平(25%), 长边=垂直(35%)
                layout[s1] = (x + w*0.25, y + h*0.35, w*0.5, h*0.9, x+2, y+2, int(x+w/2)-1, y+h-2)
                layout[s2] = (x + w*0.75, y + h*0.35, w*0.5, h*0.9, int(x+w/2)+1, y+2, x+w-2, y+h-2)
            elif c1[0] == c2[0]:  # 同列 → 上下分
                # 长边=水平(35%), 短边=垂直(25%)
                layout[s1] = (x + w*0.35, y + h*0.25, w*0.9, h*0.5, x+2, y+2, x+w-2, int(y+h/2)-1)
                layout[s2] = (x + w*0.35, y + h*0.75, w*0.9, h*0.5, x+2, int(y+h/2)+1, x+w-2, y+h-2)
            else:  # 对角 → 三角背景
                layout[s1] = (x + w*0.25, y + h*0.25, w*0.6, h*0.6, "poly", x, y, x+w, y, x, y+h)
                layout[s2] = (x + w*0.75, y + h*0.75, w*0.6, h*0.6, "poly", x+w, y, x+w, y+h, x, y+h)

        elif n == 3:
            missing = [s for s in ["tap","hold","dt","dh"] if s not in active_slots][0]
            hf = h * 0.55
            if missing == "dh":
                layout["tap"]  = (x + w*0.25, y + h*0.25, w*0.45, h*0.4,  x+2, y+2, int(x+w/2)-1, int(y+hf))
                layout["hold"] = (x + w*0.75, y + h*0.25, w*0.45, h*0.4,  int(x+w/2)+1, y+2, x+w-2, int(y+hf))
                layout["dt"]   = (x + w*0.35, y + h*0.75, w*0.85, h*0.5,  x+2, int(y+hf)+1, x+w-2, y+h-2)
            elif missing == "dt":
                layout["tap"]  = (x + w*0.25, y + h*0.25, w*0.45, h*0.4,  x+2, y+2, int(x+w/2)-1, int(y+hf))
                layout["hold"] = (x + w*0.75, y + h*0.25, w*0.45, h*0.4,  int(x+w/2)+1, y+2, x+w-2, int(y+hf))
                layout["dh"]   = (x + w*0.65, y + h*0.75, w*0.85, h*0.5,  x+2, int(y+hf)+1, x+w-2, y+h-2)
            elif missing == "hold":
                layout["tap"]  = (x + w*0.35, y + h*0.25, w*0.85, h*0.45, x+2, y+2, x+w-2, int(y+h/2)-1)
                layout["dt"]   = (x + w*0.25, y + h*0.75, w*0.45, h*0.45, x+2, int(y+h/2)+1, int(x+w/2)-1, y+h-2)
                layout["dh"]   = (x + w*0.75, y + h*0.75, w*0.45, h*0.45, int(x+w/2)+1, int(y+h/2)+1, x+w-2, y+h-2)
            elif missing == "tap":
                layout["hold"] = (x + w*0.65, y + h*0.25, w*0.85, h*0.45, x+2, y+2, x+w-2, int(y+h/2)-1)
                layout["dt"]   = (x + w*0.25, y + h*0.75, w*0.45, h*0.45, x+2, int(y+h/2)+1, int(x+w/2)-1, y+h-2)
                layout["dh"]   = (x + w*0.75, y + h*0.75, w*0.45, h*0.45, int(x+w/2)+1, int(y+h/2)+1, x+w-2, y+h-2)

        elif n == 4:
            hf = h / 2; wf = w / 2
            layout["tap"]  = (x + w*0.25, y + h*0.25, w*0.45, h*0.45, x+2, y+2, int(x+wf)-1, int(y+hf)-1)
            layout["hold"] = (x + w*0.75, y + h*0.25, w*0.45, h*0.45, int(x+wf)+1, y+2, x+w-2, int(y+hf)-1)
            layout["dt"]   = (x + w*0.25, y + h*0.75, w*0.45, h*0.45, x+2, int(y+hf)+1, int(x+wf)-1, y+h-2)
            layout["dh"]   = (x + w*0.75, y + h*0.75, w*0.45, h*0.45, int(x+wf)+1, int(y+hf)+1, x+w-2, y+h-2)

        return layout

        return layout

    # ── 键盘图标签更新 ──
    def _refresh_kb_labels(self):
        if not hasattr(self, '_layer_kb_rects'):
            return
        idx = self._current_layer_idx
        trees = self._layer_key_trees[idx] if idx < len(self._layer_key_trees) else None
        # 收集每键所有 4 个值
        key_vals = {}
        if trees:
            for r in trees["vars"]:
                k = r[0].get().strip()
                tap_v = r[1].get() if len(r) >= 2 else ""
                hold_v = r[2].get() if len(r) >= 3 else ""
                dt_v = r[4].get() if len(r) >= 5 else ""
                dh_v = r[6].get() if len(r) >= 7 else ""
                if k:
                    key_vals[k] = {"tap": tap_v, "hold": hold_v, "dt": dt_v, "dh": dh_v}

        # 批量隐藏所有标签和输出槽位（tag 批量操作，替代逐键循环）
        self._layer_kb_canvas.itemconfigure("key_label", state="hidden")
        self._layer_kb_canvas.itemconfigure("key_output", state="hidden")

        for kid in self._layer_kb_rects:
            vals = key_vals.get(kid, {})
            slots = self._layer_kb_outputs.get(kid)
            active = [k for k in ["tap","hold","dt","dh"] if vals.get(k)]

            # 有非空值但 tap 缺失 → 用键本身作为默认 tap
            if not vals.get("tap") and any(vals.get(sk) for sk in ["hold","dt","dh"]):
                vals["tap"] = "{" + kid + "}"
                active.insert(0, "tap")

            # 先清理旧的槽位背景（无论本层是否有值）
            for _old_id in self._layer_kb_slot_bgs.pop(kid, []):
                self._layer_kb_canvas.delete(_old_id)

            if not active:
                # 无输出值 → 显示物理标签
                for tid in self._layer_kb_labels.get(kid, ()):
                    self._layer_kb_canvas.itemconfigure(tid, state="normal")
                continue

            # 计算布局
            r = self._layer_kb_canvas.coords(self._layer_kb_rects[kid])
            if not r or len(r) < 4:
                continue
            x, y = r[0], r[1]
            w, h = r[2] - r[0], r[3] - r[1]
            layout = self._calc_slot_layout(x, y, w, h, active)

            # 绘制槽位背景（仅 n>=2 时）
            _slot_colors = {
                "tap": "#DCDAF5", "hold": "#CECBF6",
                "dt":  "#E5E3F8", "dh":  "#C3C0ED",
            }
            _new_bgs = []
            for sk in active:
                if sk not in layout:
                    continue
                bg_data = layout[sk][4:]
                if len(bg_data) >= 3 and bg_data[0] == "poly":
                    # 多边形背景（对角分割）
                    _, x1, y1, x2, y2, x3, y3 = bg_data
                    _id = self._layer_kb_canvas.create_polygon(
                        int(x1), int(y1), int(x2), int(y2), int(x3), int(y3),
                        fill=_slot_colors.get(sk, "#DCDAF5"),
                        outline="", tags=("key", "key_slot_bg", kid))
                else:
                    bx1, by1, bx2, by2 = bg_data
                    _id = self._layer_kb_canvas.create_rectangle(
                        int(bx1), int(by1), int(bx2), int(by2),
                        fill=_slot_colors.get(sk, "#DCDAF5"),
                        outline="", tags=("key", "key_slot_bg", kid))
                # 将本键的标签和输出文本移到背景上方
                for _st in slots.values():
                    self._layer_kb_canvas.lift(_st)
                for _lt in self._layer_kb_labels.get(kid, ()):
                    self._layer_kb_canvas.lift(_lt)
                _new_bgs.append(_id)
            self._layer_kb_slot_bgs[kid] = _new_bgs

            # 填充每个槽位
            for sk in active:
                st = slots.get(sk)
                if not st or sk not in layout:
                    continue
                cx, cy, mw, mh, *_ = layout[sk]
                raw = vals[sk]
                fmt = self._fmt_output(raw)
                if isinstance(fmt, tuple):
                    display, color = fmt
                else:
                    display, color = fmt, "#534AB7"
                self._layer_kb_canvas.coords(st, int(cx), int(cy))
                self._layer_kb_canvas.itemconfigure(st, text=display,
                    fill=color, state="normal")
                # 自适应字号（预构建跳过 bbox 调用）
                if not getattr(self, '_prebuilding', False):
                    self._fit_output_font(kid, sk, display, cx, cy, int(mw), int(mh))

        # 着色 rect（与 _refresh_kb_colors 等价，省掉一次 111 键遍历）
        selected = getattr(self, '_selected_kb_key', None)
        tgt = getattr(self, "_edit_target", None)
        ln = None
        if tgt is not None:
            ln = trees.get("name") if trees else ("base" if idx == 0 else f"fn{idx}")
        for kid, rect_id in self._layer_kb_rects.items():
            _has_slot_bg = bool(self._layer_kb_slot_bgs.get(kid))
            if kid == selected:
                fill, outline, width = "#AFA9EC", "#534AB7", 2
                if tgt is not None and ln is not None:
                    ident = ("layers", ln, kid)
                    outline, width = self._badge_border(ident, owned_w=4, inherited_w=2)
            elif kid in key_vals:
                fill = "#D3D1C7" if _has_slot_bg else "#CECBF6"
                outline, width = "#AFA9EC", 1
                if tgt is not None and ln is not None:
                    ident = ("layers", ln, kid)
                    outline, width = self._badge_border(ident, owned_w=4, inherited_w=2)
            else:
                fill, outline, width = "#D3D1C7", "#B4B2A9", 1
            self._layer_kb_canvas.itemconfig(rect_id,
                fill=fill, outline=outline, width=width)

    # ── 缩放后更新字号 ──
    def _update_kb_fonts(self, scale):
        fm = max(7, int(self._kb_font_main_base * scale))
        fs = max(7, int(self._kb_font_sub_base * scale))
        fp = max(7, int(10 * scale))
        # 调整 Font 对象尺寸（所有引用该 Font 的 text item 立即生效）
        self._kb_font_main_font.configure(size=fm)
        self._kb_font_sub_font.configure(size=fs)
        self._kb_font_pause_font.configure(size=fp)
        # 重新设置 width（text item 的换行宽度由 width 控制，不随 Font 自动缩放）
        for kid, tids in self._layer_kb_labels.items():
            r = self._layer_kb_canvas.coords(self._layer_kb_rects[kid])
            kw = max(1, int(r[2] - r[0]) - 6)
            for i, tid in enumerate(tids):
                f = self._kb_font_pause_font if kid in ("Pause","NumLock") else (self._kb_font_main_font if i == 0 else self._kb_font_sub_font)
                self._layer_kb_canvas.itemconfigure(tid, font=f, width=kw)
        # 输出槽位：重新计算布局（字号已因 scale 变化，bbox 能适配）
        self._refresh_kb_labels()

    # ── 自适应输出文字字号（单槽）──
    def _fit_output_font(self, kid, slot_key, text, cx, cy, max_w, max_h):
        """在指定位置以合适字号显示 text，保证不超出 (max_w, max_h)。"""
        slots = self._layer_kb_outputs.get(kid)
        if not slots or slot_key not in slots:
            return
        ot = slots[slot_key]
        old_state = self._layer_kb_canvas.itemcget(ot, "state")
        self._layer_kb_canvas.itemconfigure(ot, state="normal")
        self._layer_kb_canvas.coords(ot, cx, cy)
        # 二分搜索最适字号（替代逐级递减的线性扫描，bbox 调用从 ~9 降到 ~4）
        base = max(8, int(self._kb_font_out_base * self._kb_scale))
        lo, hi, best = 6, base, 6
        while lo <= hi:
            mid = (lo + hi) // 2
            self._layer_kb_canvas.itemconfigure(ot, text=text,
                font=("Consolas", mid, "bold"), width=0)
            bbox = self._layer_kb_canvas.bbox(ot)
            if not bbox:
                break
            tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
            if tw <= max_w + 2 and th <= max_h + 2:
                best = mid
                lo = mid + 1  # 尝试更大字号
            else:
                hi = mid - 1  # 缩小字号
        self._layer_kb_canvas.itemconfigure(ot,
            font=("Consolas", best, "bold"), width=0)
        self._layer_kb_canvas.itemconfigure(ot, state=old_state)

    # ── 键盘图着色 ──
    def _refresh_kb_colors(self):
        if not hasattr(self, '_layer_kb_rects'):
            return
        idx = self._current_layer_idx
        trees = self._layer_key_trees[idx] if idx < len(self._layer_key_trees) else None
        mapped_keys = set()
        mapped_out = {}
        if trees:
            for r in trees["vars"]:
                k = r[0].get().strip()
                tap_v  = r[1].get() if len(r) >= 2 else ""
                hold_v = r[2].get() if len(r) >= 3 else ""
                dt_v   = r[4].get() if len(r) >= 5 else ""
                dh_v   = r[6].get() if len(r) >= 7 else ""
                if k and (tap_v or hold_v or dt_v or dh_v):
                    mapped_keys.add(k)
                    mapped_out[k] = tap_v
        selected = getattr(self, '_selected_kb_key', None)

        # 设备编辑模式下计算覆盖/继承集合，用于按键边框着色
        tgt = getattr(self, "_edit_target", None)
        overrides = inherited = set()
        ln = None
        if tgt is not None:
            overrides, inherited = self._badge_sets()
            ln = trees.get("name") if trees else ("base" if idx == 0 else f"fn{idx}")

        for kid, rect_id in self._layer_kb_rects.items():
            # 有槽位背景时用底色，让 slot BG 显色
            _has_slot_bg = bool(self._layer_kb_slot_bgs.get(kid))
            if kid == selected:
                fill = "#AFA9EC"
                outline = "#534AB7"
                width = 2
                # 选中键也显示覆盖/继承边框色（比默认紫色更直观）
                if tgt is not None and ln is not None:
                    ident = ("layers", ln, kid)
                    if ident in overrides:
                        outline = _THEME["override"]
                    elif ident in inherited:
                        outline = _THEME["inherit"]
                self._layer_kb_canvas.itemconfig(rect_id,
                    fill=fill, outline=outline, width=width)
            elif kid in mapped_keys:
                fill = "#D3D1C7" if _has_slot_bg else "#CECBF6"
                outline = "#AFA9EC"
                width = 1
                if tgt is not None and ln is not None:
                    ident = ("layers", ln, kid)
                    outline, width = self._badge_border(ident, owned_w=4, inherited_w=2)
                self._layer_kb_canvas.itemconfig(rect_id,
                    fill=fill, outline=outline, width=width)
            else:
                self._layer_kb_canvas.itemconfig(rect_id,
                    fill="#D3D1C7", outline="#B4B2A9", width=1)

    # ── 宏列表（两列：左列+右列）──
    def _refresh_macro_table(self):
        if not hasattr(self, '_macro_text'):
            return
        idx = self._current_layer_idx
        if idx >= len(self._layer_key_trees):
            return
        trees = self._layer_key_trees[idx]
        entries = []
        for r in trees["vars"]:
            k = r[0].get().strip()
            if not k:
                continue
            tap_v  = r[1].get() if len(r) >= 2 else ""
            hold_v = r[2].get() if len(r) >= 3 else ""
            dt_v   = r[4].get() if len(r) >= 5 else ""
            dh_v   = r[6].get() if len(r) >= 7 else ""
            if tap_v or hold_v or dt_v or dh_v:
                entries.append((k, _norm_key(k), tap_v, hold_v, dt_v, dh_v))

        def _sort_key(item):
            k = item[1].lower()
            if k.isdigit(): return (0, k)
            if len(k) == 1 and k.isalpha(): return (1, k)
            if k.startswith("f") and k[1:].isdigit(): return (2, k)
            if k in {"up","down","left","right","home","end","pgup","pgdn",
                     "insert","delete","backspace","tab","escape","enter",
                     "space","printscreen","scrolllock","pause","capslock"}:
                return (3, k)
            if k in {"lcontrol","rcontrol","lshift","rshift","lalt","ralt",
                     "lwin","rwin","appskey"}: return (4, k)
            if k.startswith("numpad"): return (5, k)
            return (6, k)
        entries.sort(key=_sort_key)

        # 拆分：奇数索引→左列，偶数索引→右列
        left_entries = entries[::2]
        right_entries = entries[1::2]

        prev_sel = self._selected_kb_key
        self._macro_keys = []
        self._macro_norm = []
        self._macro_keys_r = []
        self._macro_norm_r = []

        # 宏列表保持默认颜色，覆盖/继承颜色统一画在键盘图按键边框上
        def _fill_text(text_widget, entry_list, keys_list, norm_list):
            text_widget.configure(state="normal")
            text_widget.delete("1.0", "end")
            text_widget.tag_remove("hl", "1.0", "end")
            if not entry_list:
                text_widget.insert("end", "（无映射）\n")
            else:
                for raw_k, norm_k, tap_v, hold_v, dt_v, dh_v in entry_list:
                    text_widget.insert("end", norm_k.ljust(10) + "\t", ("val",))
                    first = True
                    if tap_v:
                        if not first: text_widget.insert("end", "  ", ("val",))
                        text_widget.insert("end", "T:", ("prefix", "val"))
                        text_widget.insert("end", tap_v, ("val",))
                        first = False
                    if hold_v:
                        if not first: text_widget.insert("end", "  ", ("val",))
                        text_widget.insert("end", "H:", ("prefix", "val"))
                        text_widget.insert("end", hold_v, ("val",))
                        first = False
                    if dt_v:
                        if not first: text_widget.insert("end", "  ", ("val",))
                        text_widget.insert("end", "D:", ("prefix", "val"))
                        text_widget.insert("end", dt_v, ("val",))
                        first = False
                    if dh_v:
                        if not first: text_widget.insert("end", "  ", ("val",))
                        text_widget.insert("end", "DH:", ("prefix", "val"))
                        text_widget.insert("end", dh_v, ("val",))
                    text_widget.insert("end", "\n", ("val",))
                    keys_list.append(raw_k)
                    norm_list.append(norm_k)
            text_widget.configure(state="disabled")

        _fill_text(self._macro_text, left_entries, self._macro_keys, self._macro_norm)
        _fill_text(self._macro_text_r, right_entries, self._macro_keys_r, self._macro_norm_r)

        # 恢复高亮
        if prev_sel:
            self._macro_text.after(10, lambda k=prev_sel: self._highlight_macro_key(k))

    def _highlight_macro_key(self, key_id):
        """用自定义 hl tag 高亮宏列表的某个键名所在行。

        不用 "sel" tag（tk.Text 内建 sel 受焦点状态影响，未 focus 时不渲染）。
        hl tag 完全独立于焦点，直接在 widget 上覆盖背景色。
        匹配时通过 _norm_key() 双向 normalize（处理大小写/缩写变体）。
        """
        text_l = getattr(self, "_macro_text", None)
        text_r = getattr(self, "_macro_text_r", None)
        if text_l is None:
            return
        try:
            target = _norm_key(key_id).lower() if key_id else ""
        except Exception:
            target = (key_id or "").lower()
        # 清旧 hl（两列）
        text_l.tag_remove("hl", "1.0", "end")
        text_r.tag_remove("hl", "1.0", "end")
        if not target:
            return
        # 搜索左列
        for i, kn in enumerate(getattr(self, "_macro_norm", [])):
            if (kn or "").lower() == target:
                line = i + 1
                try: text_l.see(f"{line}.0"); text_l.tag_add("hl", f"{line}.0", f"{line}.0 lineend+1c"); text_l.tag_raise("hl")
                except Exception: pass
                return
        # 搜索右列
        for i, kn in enumerate(getattr(self, "_macro_norm_r", [])):
            if (kn or "").lower() == target:
                line = i + 1
                try: text_r.see(f"{line}.0"); text_r.tag_add("hl", f"{line}.0", f"{line}.0 lineend+1c"); text_r.tag_raise("hl")
                except Exception: pass
                return

    def _on_macro_click(self, event=None, col="left"):
        """点击宏列表某一行：commit entry 修改 → 高亮该行 → 选中对应按键。"""
        for e in self._kb_entries:
            self._commit_entry(e)
        if col == "left":
            text = self._macro_text
            keys = self._macro_keys
        else:
            text = self._macro_text_r
            keys = self._macro_keys_r
        # 把鼠标 event.x / event.y 换算到 widget 行号
        idx = text.index(f"@{event.x},{event.y}")
        line = int(idx.split(".")[0]) if idx else 0
        if 1 <= line <= len(keys):
            # 高亮（两列都清除，只高亮当前列）
            self._macro_text.tag_remove("hl", "1.0", "end")
            self._macro_text_r.tag_remove("hl", "1.0", "end")
            text.tag_add("hl", f"{line}.0", f"{line}.0 lineend+1c")
            text.tag_raise("hl")
            # 选键
            self._select_kb_key(keys[line - 1])

    def _on_entry_double_click(self, event, entry, var_index):
        """输入框双击：弹出更大的输入窗口，便于编辑该字段的完整内容。"""
        entry = self._kb_entries[var_index]
        # 若当前是 hint 态（用户从未输入过），用空串作为初始值
        if getattr(entry, "_showing_hint", False):
            init_val = ""
        else:
            init_val = entry.get()

        field_name = self._hint_labels[var_index]

        def _apply(text):
            # 写到 entry 显示（与 _insert_preset 同套风格）
            entry.configure(textvariable=None)
            entry.delete(0, "end")
            entry.insert(0, text)
            entry.configure(text_color=_THEME["text_dark"],
                            font=("Consolas", 12))
            entry._showing_hint = not bool(text.strip())
            entry._modified = True
            # 写回 StringVar
            if entry._var is not None:
                try:
                    entry._var.set(text)
                except Exception:
                    pass
            self._schedule_autosave()
            self._refresh_kb_labels()
            self._refresh_macro_table()

        dlg = LargeInputDialog(self, f"{field_name} 输入", init_val, on_confirm=_apply)
        # 大窗口同样接收按键速查按钮的插入
        self._dlg_target = dlg.textbox
        dlg.wait_window()
        self._dlg_target = None

    # ── 层标签（显示 0=baseLayer, 1=fn1, 2=fn2...）──
    def _refresh_layer_tabs(self):
        for w in self._layer_tab_frame.winfo_children():
            w.destroy()
        self._layer_tab_btns = []
        for i in range(len(self._layer_key_trees)):
            text = "0" if i == 0 else str(i)  # 基础层 → "0", fn层 → "1","2"...
            btn = ctk.CTkButton(self._layer_tab_frame, text=text,
                command=lambda idx=i: self._switch_layer(idx),
                fg_color=_THEME["card"], text_color=_THEME["text_dark"],
                corner_radius=8, width=28, height=28,
                font=("Microsoft YaHei", 12, "bold"))
            btn.pack(side="left", padx=(0, 2))
            self._layer_tab_btns.append(btn)
        self._update_layer_tab_style()
        if len(self._layer_key_trees) <= 1:
            self._del_layer_btn.configure(state="disabled")
        else:
            self._del_layer_btn.configure(state="normal")
        self.update_idletasks()
        self._update_layer_arrows()
        # _fit_layer_canvas_height 移到 _layer_tab_ready=True 之后，
        # 避免在 f.pack() 前读取不准的 winfo_reqheight 导致高度二次调整。

    def _fit_layer_canvas_height(self):
        """让层标签画布高度精确匹配按钮实际渲染高度。

        按钮的 height=28 会随系统 DPI 缩放放大，而 tk.Canvas 的 height
        是原始像素、不被缩放，固定高度会导致高 DPI 下按钮被纵向裁切
        （只露出一半）。这里按按钮真实像素高度动态设置画布高度。
        """
        canvas = getattr(self, "_layer_scroll", None)
        frame = getattr(self, "_layer_tab_frame", None)
        if canvas is None or frame is None:
            return
        try:
            h = frame.winfo_reqheight()          # 请求高度，初始化阶段即可用
            if h < 10:
                h = frame.winfo_height()          # 已映射后用实际高度
            if h < 10:                            # 兜底：按缩放系数估算
                try:
                    scaling = ctk.ScalingTracker.get_window_scaling(self)
                except Exception:
                    scaling = 1.0
                h = int(28 * scaling)
            canvas.configure(height=h + 6)       # 上下各 3px 呼吸空间
        except Exception:
            pass

    def _update_layer_tab_style(self):
        for i, btn in enumerate(self._layer_tab_btns):
            if i == self._current_layer_idx:
                btn.configure(fg_color=_THEME["blue"], text_color="white")
            else:
                btn.configure(fg_color=_THEME["card"], text_color=_THEME["text_dark"])

    # ── 层标签横向滚动（箭头 + 滚轮）──
    def _scroll_layer_tabs(self, direction):
        """direction: -1 向左, +1 向右。每次滚动一个步长（xscrollincrement）。"""
        if not hasattr(self, "_layer_scroll"):
            return
        self._layer_scroll.xview_scroll(-1 if direction < 0 else 1, "units")
        self._update_layer_arrows()

    def _on_layer_wheel(self, event):
        """鼠标滚轮横向滚动层标签。"""
        if not hasattr(self, "_layer_scroll"):
            return
        units = -1 if event.delta > 0 else 1
        self._layer_scroll.xview_scroll(units, "units")
        self._update_layer_arrows()
        return "break"

    def _update_layer_arrows(self):
        """根据内容是否溢出，显示/隐藏左右箭头，并在到达边界时禁用。"""
        canvas = getattr(self, "_layer_scroll", None)
        if canvas is None:
            return
        try:
            canvas.update_idletasks()
            total = self._layer_tab_frame.winfo_width()
            visible = canvas.winfo_width()
        except Exception:
            return
        # 内容未溢出：隐藏箭头
        if total <= visible or total <= 1:
            if self._layer_left_arrow.winfo_ismapped():
                self._layer_left_arrow.pack_forget()
            if self._layer_right_arrow.winfo_ismapped():
                self._layer_right_arrow.pack_forget()
            canvas.xview_moveto(0.0)
            return
        # 内容溢出：显示箭头，按边界禁用
        if not self._layer_left_arrow.winfo_ismapped():
            self._layer_left_arrow.pack(side="left", padx=(0, 2))
        if not self._layer_right_arrow.winfo_ismapped():
            self._layer_right_arrow.pack(side="left", padx=(2, 4))
        try:
            first, last = canvas.xview()
        except Exception:
            first, last = 0.0, 1.0
        self._layer_left_arrow.configure(state="disabled" if first <= 0.001 else "normal")
        self._layer_right_arrow.configure(state="disabled" if last >= 0.999 else "normal")

    def _reset_kb_edits(self):
        """重置编辑栏：清空所有 var + 切回 hint 显示。"""
        self._kb_vars = [None] * 7
        self._kb_tap_var = self._kb_hold_var = self._kb_ht_var = None
        self._kb_dt_var = self._kb_dtt_var = self._kb_dh_var = self._kb_dht_var = None
        self._active_entry = None
        for e in self._kb_entries:
            self._load_entry(e, None)

    def _switch_layer(self, idx):
        # 先 commit 当前 entry 修改，再切层
        self._commit_all_entries()
        self._current_layer_idx = idx
        self._selected_kb_key = None
        self._reset_kb_edits()
        self._kb_key_label.configure(text="\u2014")
        self._update_layer_tab_style()
        self._refresh_layer_controls()
        self._refresh_kb_labels()
        self._refresh_kb_colors()
        self._refresh_macro_table()

    def _refresh_layer_controls(self):
        for w in self._layer_ctrl_frame.winfo_children():
            w.destroy()
        idx = self._current_layer_idx
        if idx == 0:
            # 基础层：无切换键，只显示标签
            ctk.CTkLabel(self._layer_ctrl_frame, text="基础层 - 默认按键映射",
                         font=("Microsoft YaHei", 13, "bold"),
                         text_color=_THEME["text_mid"]).pack(side="left", padx=(0, 12))
        else:
            ctk.CTkLabel(self._layer_ctrl_frame, text="功能层 - 点击键盘键进行编辑",
                         font=("Microsoft YaHei", 13, "bold"),
                         text_color=_THEME["text_mid"]).pack(side="left", padx=(0, 12))
        # 有内容时才 pack
        if not self._layer_ctrl_frame.winfo_ismapped():
            self._layer_ctrl_frame.pack(side="top", anchor="w", padx=10, pady=(2, 2))

    # ── 层卡片重建 ─────────────────────────
    def _reload_layer_cards(self):
        """从 self.cfg 重建层数据"""
        if not hasattr(self, '_layer_key_trees'):
            return
        self._switch_key_vars.clear()
        self._layer_hold_vars.clear()
        self._layer_block_vars.clear()
        self._layer_key_trees.clear()

        cfg = self.cfg.get("tapDance", {})
        base_layer_map = cfg.get("base", {})
        _fn_keys = sorted([k for k in cfg if k.startswith("fn")], key=lambda x: int(x[2:] or 0))
        if not _fn_keys:
            _fn_keys = ["fn1", "fn2"]
        # 索引 0 = base 层
        self._init_layer_data(0, {"name": "base", "keyMap": base_layer_map})
        for i, fn_name in enumerate(_fn_keys):
            self._init_layer_data(i + 1, {"name": fn_name, "keyMap": cfg.get(fn_name, {})})

        self._current_layer_idx = 0
        self._selected_kb_key = None
        # 层 tab 未构建时跳过 UI 刷新（首次点击时由 _build_layer_tab 重绘）
        if not hasattr(self, '_layer_tab_frame'):
            return
        self._refresh_layer_tabs()
        self._refresh_layer_controls()
        self._refresh_kb_labels()
        self._refresh_kb_colors()
        self._refresh_macro_table()

    # ── 层增删 ─────────────────────────────
    def _add_layer(self):
        """在末尾追加一个空层（层名取最小未占用的 fn 序号，避免与已有层冲突）"""
        cfg = self.cfg.get("tapDance", {})
        used = {int(k[2:]) for k in cfg if k.startswith("fn") and k[2:].isdigit()}
        n = 1
        while n in used:
            n += 1
        new_name = f"fn{n}"
        cfg[new_name] = {}
        self._init_layer_data(len(self._layer_key_trees), {"name": new_name, "keyMap": {}})
        self._refresh_layer_tabs()
        self._schedule_autosave()

    def _del_layer(self):
        """删除当前层（不能删除 base）"""
        if len(self._layer_key_trees) <= 1:
            messagebox.showinfo("提示", "至少需要保留一个层", parent=self)
            return
        idx = self._current_layer_idx
        if idx == 0:
            messagebox.showinfo("提示", "不能删除基础层", parent=self)
            return
        name = self._layer_key_trees[idx].get("name") or f"fn{idx}"
        if not messagebox.askyesno("确认", f"删除{name}？", parent=self):
            return
        self._switch_key_vars.pop(idx)
        self._layer_hold_vars.pop(idx)
        self._layer_block_vars.pop(idx)
        self._layer_key_trees.pop(idx)
        cfg = self.cfg.get("tapDance", {})
        cfg.pop(name, None)
        if self._current_layer_idx >= len(self._layer_key_trees):
            self._current_layer_idx = len(self._layer_key_trees) - 1
        self._selected_kb_key = None
        self._refresh_layer_tabs()
        self._refresh_layer_controls()
        self._refresh_kb_colors()
        self._schedule_autosave()

    # ── 弹出大窗口输入 ─────────────────────────
    def _popup_large_input(self, string_var):
        """双击输入框时弹出大窗口进行输入（写回 StringVar）"""
        def _apply(text):
            string_var.set(text.strip())

        dlg = LargeInputDialog(self, "编辑键值", string_var.get(), on_confirm=_apply)
        # 大窗口同样接收按键速查按钮的插入
        self._dlg_target = dlg.textbox
        dlg.wait_window()
        self._dlg_target = None

    # ── 收集层配置（从 UI 控件）──────────────
    def _collect_layer_cfg(self):
        """收集层数据为扁平格式 {base: {...}, fn1: {...}, ...}"""
        if self._switch_key_vars and self._layer_key_trees:
            out = {}
            for i, trees_entry in enumerate(self._layer_key_trees):
                key_map = {}
                for r in trees_entry.get("vars", []):
                    if len(r) < 2:
                        continue
                    k = r[0].get().strip()
                    tap = r[1].get()
                    hold = r[2].get() if len(r) >= 3 else ""
                    ht = r[3].get().strip() if len(r) >= 4 else ""
                    dt = r[4].get() if len(r) >= 5 else ""
                    dtt = r[5].get().strip() if len(r) >= 6 else ""
                    dh = r[6].get() if len(r) >= 7 else ""
                    dht = r[7].get().strip() if len(r) >= 8 else ""
                    if k and (tap or hold or ht or dt or dtt or dh or dht):
                        key_map[k] = {"tap": tap, "hold": hold, "ht": ht,
                                      "dt": dt, "dtt": dtt, "dh": dh, "dht": dht}
                ln = trees_entry.get("name") or ("base" if i == 0 else f"fn{i}")
                out[ln] = key_map
            return out
        # 回落：从 self.cfg 读取（已是旧格式，需转换）
        return self._tapdance_from_legacy(self.cfg.get("tapDance", {}))

    @staticmethod
    def _is_switchkey_value(hold_val):
        """判断 hold 值是否为 switchkey（层键或修饰键）"""
        import re
        if not hold_val:
            return False
        return bool(re.match(
            r'^\{(?:[fb]n\d+|shift|ctrl|alt|win|lshift|rshift|lctrl|rctrl|lalt|ralt|lwin|rwin)\}$',
            hold_val
        ))

    def _autofill_switchkey_to_upper_layers(self, base_layer_map, layer_defs):
        """将 base 层的 switchkey hold 自动填充到各上层同键空 hold 位
        仅对尚未自动填充过的 (layer_name, phys_key) 生效，避免覆盖用户手动修改"""
        for phys_key, entry in base_layer_map.items():
            hold_val = entry.get("hold", "").strip()
            if not hold_val or not self._is_switchkey_value(hold_val):
                continue
            for layer_name, key_map in layer_defs:
                # 已处理过则跳过
                tag = (layer_name, phys_key)
                if tag in self._autofilled_switchkeys:
                    continue
                # 该键在上层存在且 hold 为空 → 自动填入
                if phys_key in key_map:
                    existing_hold = key_map[phys_key].get("hold", "").strip()
                    if not existing_hold:
                        key_map[phys_key]["hold"] = hold_val
                        self._autofilled_switchkeys.add(tag)

    def _update_device_checkboxes_state(self):
        """根据 _per_device_var 控制设备行外观（置灰/恢复）"""
        per_dev = self._per_device_var.get()
        state = "disabled" if not per_dev else "normal"
        label_color = _THEME["text_disabled"] if not per_dev else _THEME["text_light"]
        entry_color = _THEME["text_disabled"] if not per_dev else _THEME["text_mid"]
        # 复选框被禁时改背景色成灰色
        cb_fg = _THEME["border"] if not per_dev else _THEME["blue"]
        cb_border = _THEME["text_disabled"] if not per_dev else _THEME["blue"]
        cb_text_color = _THEME["text_disabled"] if not per_dev else _THEME["blue"]

        for row in self._device_rows:
            if "checkbox" in row:
                row["checkbox"].configure(state=state)
                if not per_dev:
                    row["checkbox"].configure(
                        border_color=_THEME["text_disabled"],
                        fg_color=_THEME["text_disabled"])
                else:
                    row["checkbox"].configure(
                        border_color=_THEME["blue"],
                        fg_color=_THEME["blue"])
            if "name_entry" in row:
                row["name_entry"].configure(text_color=entry_color)
            if "vid_pid_label" in row:
                row["vid_pid_label"].configure(text_color=label_color)
            if "icon" in row:
                row["icon"].configure(text_color=label_color)
            if "domain" in row and row["domain"] is not None:
                row["domain"].configure(
                    state="disabled" if not per_dev else "normal")

    # ── 引擎安装 ─────────────────────────────────

    def _install_rust(self):
        """Rust 引擎部署：复制 exe 到引擎目录"""
        from tkinter import messagebox
        from lib.config import RUST_ENGINE_PATH
        import shutil, threading

        def run():
            try:
                rust_dir = os.path.dirname(RUST_ENGINE_PATH)
                os.makedirs(rust_dir, exist_ok=True)
                src = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                   "anykey-engine", "target", "release", "anykey-engine.exe")
                if not os.path.exists(src):
                    src = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                       "anykey-engine", "target", "debug", "anykey-engine.exe")
                if os.path.exists(src):
                    shutil.copy2(src, RUST_ENGINE_PATH)
                    self.after(0, lambda: messagebox.showinfo("部署完成", "Rust 引擎已部署", parent=self))
                else:
                    self.after(0, lambda: messagebox.showerror("部署失败",
                        "未找到编译好的 anykey-engine.exe，请先运行 cargo build --release", parent=self))
            except Exception as e:
                self.after(0, lambda: messagebox.showerror("部署失败", str(e), parent=self))

        threading.Thread(target=run, daemon=True).start()

    def _toggle_identify(self):
        """开始/停止设备识别（通过 filter driver 监听按键）"""
        if self._identifying:
            self._stop_identify()
            return

        self._identifying = True
        self._identify_btn.configure(text="识别中...(点击停止)", fg_color=_THEME["red"])
        # 识别期间禁用刷新：识别线程持有驱动句柄，此时主线程再发 ENUM_DEVICES
        # 会在同一设备对象上串行 → 死锁卡死。
        if getattr(self, "_refresh_btn", None) is not None:
            self._refresh_btn.configure(state="disabled")

        import threading, time
        def listen():
            from lib.driver import get_driver
            fd = get_driver()
            if not fd.open():
                self.after(0, lambda: self._stop_identify())
                return
            # 开启透传捕获：驱动在不拦截（不吞键）的前提下把输入镜像进队列，
            # 这样识别时键盘/鼠标仍能正常操作，GUI 又能轮询到事件。
            fd.set_capture(True)
            try:
                while self._identifying:
                    # 键盘事件
                    for evt in fd.poll_input(timeout_ms=60):
                        if not self._identifying: break
                        is_down = (evt["flags"] & 1) == 0
                        if is_down:
                            kn = evt.get("key_name", "")
                            self.after(0, lambda d=evt["device_id"], k=kn: self._on_key_identified(d, k))
                    # 鼠标事件
                    for evt in fd.poll_mouse_input(timeout_ms=60):
                        if not self._identifying: break
                        if evt["button_flags"] & 0x1:  # LEFT_DOWN
                            self.after(0, lambda d=evt["device_id"]: self._on_key_identified(d, "LBtn"))
                        elif evt["button_flags"] & 0x4:  # RIGHT_DOWN
                            self.after(0, lambda d=evt["device_id"]: self._on_key_identified(d, "RBtn"))
                    time.sleep(0.02)
            finally:
                fd.set_capture(False)

        threading.Thread(target=listen, daemon=True).start()

    def _stop_identify(self):
        """停止设备识别"""
        self._identifying = False
        self._identify_btn.configure(text="识别", fg_color=_THEME["green"])
        if getattr(self, "_refresh_btn", None) is not None:
            self._refresh_btn.configure(state="normal")
        # 关闭透传捕获（若驱动句柄已开）
        try:
            from lib.driver import get_driver
            get_driver().set_capture(False)
        except Exception:
            pass
        for row in self._device_rows:
            row["hint"].configure(text="")
            row["frame"].configure(border_color=_THEME["border"], border_width=1)

    def _on_key_identified(self, device_id, key_name=""):
        """按键识别回调—— 按 device_id 匹配设备行，高亮该行并显示按键名"""
        if not self._identifying:
            return
        hint = f"\u2190 {key_name}" if key_name else ""
        for row in self._device_rows:
            _ids = row.get("device_ids") or {row["device"].get("device_id", 0)}
            if device_id in _ids:
                row["hint"].configure(text=hint, text_color=_THEME["green"])
                row["frame"].configure(border_color=_THEME["green"], border_width=3)
            else:
                row["hint"].configure(text="")
                row["frame"].configure(border_color=_THEME["border"], border_width=1)

    def _load_data(self):
        # combo 表格在 _build_combo_tab 末尾已调用 _populate_combo_table
        # 层数据：仅当层 tab 已构建才重建（初始化时还没构建）
        self._reload_layer_cards()

    # ── 帮助文档 ───────────────────────────────────────
    def _open_help(self):
        """打开帮助文档 help.md"""
        # frozen 模式下从 _MEIPASS 读取；开发模式从脚本目录读取
        if getattr(sys, 'frozen', False):
            base_dir = sys._MEIPASS
        else:
            base_dir = os.path.dirname(os.path.abspath(__file__))
        help_path = os.path.join(base_dir, "help.md")
        if os.path.exists(help_path):
            webbrowser.open(help_path)
        else:
            messagebox.showinfo("帮助", "帮助文档尚未创建。", parent=self)

    # ── 导出 / 导入配置 ──────────────────────────────────
    def _export_config(self):
        """导出配置到用户选择的文件"""
        path = filedialog.asksaveasfilename(
            title="导出配置",
            defaultextension=".json",
            filetypes=[("JSON 配置文件", "*.json"), ("所有文件", "*.*")],
            initialfile="anykey_config_export.json",
        )
        if not path:
            return
        cfg = self._collect_cfg()
        try:
            with open(path, "w", encoding="utf-8") as f:
                json.dump(cfg, f, ensure_ascii=False, indent=2)
            status = getattr(self, 'status_var', None)
            if status:
                status.set(f"✓ 配置已导出")
            print(f"配置已导出到: {path}")
        except Exception as e:
            messagebox.showerror("导出失败", str(e), parent=self)

    def _import_config(self):
        """从文件导入配置"""
        path = filedialog.askopenfilename(
            title="导入配置",
            filetypes=[("JSON 配置文件", "*.json"), ("所有文件", "*.*")],
        )
        if not path:
            return
        try:
            with open(path, "r", encoding="utf-8") as f:
                new_cfg = json.load(f)
            # 导入即原样采用，不再自动回填默认字段（缺失字段由各读取处 .get 兜底）
            self.cfg = new_cfg
            # 同步权威配置，否则 _config_master 仍是旧数据，后续 flush 会用旧值覆盖导入内容
            self._config_master = self._dc(new_cfg)
            save_config(self.cfg)
            # 重建 UI：销毁已缓存的所有 tab frame，重建 combo 页面
            for tab_name, frame in list(self._tab_frames.items()):
                try:
                    frame.destroy()
                except Exception:
                    pass
            self._tab_frames.clear()
            self._layer_inner = None
            self._layer_cards = []
            self._switch_key_vars = []
            self._layer_key_trees = []
            # 切换回 combo 标签（会触发重建）
            self._switch_tab("combo")
            status = getattr(self, 'status_var', None)
            if status:
                status.set("✓ 配置已导入")
            print(f"配置已从 {path} 导入")
        except Exception as e:
            messagebox.showerror("导入失败", str(e), parent=self)

    # ──────────────────────────────────────────────
    # 序列（Leader）标签页：复用 combo 同款可编辑表格
    # 列：超时(ms) | 按键序列 | 输出 | 删除
    # 末尾保留一个空白哨兵行等待输入
    # ──────────────────────────────────────────────
    def _build_sequences_tab(self, parent):
        """序列 Leader 配置标签页内容：全宽单栏"""
        parent.configure(fg_color="transparent")

        page = ctk.CTkFrame(parent, fg_color="transparent")
        page.pack(fill="both", expand=True, pady=0)

        # ── 序列 Leader（标题 + 触发键 + 公共超时）──────
        c1 = ctk.CTkFrame(page, corner_radius=CARD_RADIUS, fg_color=_THEME["card"], border_width=0)
        c1.pack(fill="both", expand=True, pady=0)

        hdr = ctk.CTkFrame(c1, fg_color="transparent")
        hdr.pack(fill="x", padx=14, pady=(12, 4))

        ctk.CTkLabel(hdr, text="序列 Leader", font=("Microsoft YaHei", 14, "bold"),
                     text_color=_THEME["text_mid"]).pack(side="left")

        # ── 右侧：公共超时 + 循环捕获 ──
        right_side = ctk.CTkFrame(hdr, fg_color="transparent")
        right_side.pack(side="right")

        # 公共超时
        self._leader_timeout_var = ctk.StringVar(
            value=str(self.cfg.get("leader", {}).get("timeoutMs", EMPTY_LEADER["timeoutMs"])))
        self._leader_timeout_var.trace_add("write", lambda *_: self._schedule_autosave())
        time_container = ctk.CTkFrame(right_side, fg_color="transparent")
        time_container.pack(side="right")
        ctk.CTkLabel(time_container, text="公共超时", font=("Microsoft YaHei", 11),
                     text_color=_THEME["text_light"]).pack(side="left", padx=(0, 4))
        ctk.CTkEntry(time_container, textvariable=self._leader_timeout_var,
                     font=("Segoe UI", 13), text_color=_THEME["text_dark"],
                     fg_color=_THEME["card"], border_color=_THEME["border"],
                     border_width=1, corner_radius=6, justify="center",
                     width=52, height=28).pack(side="left")
        ctk.CTkLabel(time_container, text="ms", font=("Microsoft YaHei", 11),
                     text_color=_THEME["text_light"]).pack(side="left", padx=(2, 12))

        # 触发键固定为 {leader}（blocking 拦截开关），GUI 不暴露输入框

        # 循环捕获（默认关，维持原版；勾选则 leader 输出键回灌驱动串联）
        self._leader_loop_capture_var = ctk.BooleanVar(
            value=self.cfg.get("leader", {}).get("loopCapture", EMPTY_LEADER["loopCapture"]))
        self._leader_loop_capture_var.trace_add("write", lambda *_: self._schedule_autosave())
        cb_lc = ctk.CTkCheckBox(right_side, text="循环捕获", variable=self._leader_loop_capture_var,
                                font=("Microsoft YaHei", 11))
        cb_lc.pack(side="right", padx=(0, 14))
        self._tooltip_on_hover(cb_lc, "勾选后 leader 输出键会回灌进匹配，驱动序列自动串联（如 12→3→123→4→…→bingo）。不勾选维持原版：输出不进栈。")

        # 说明
        ctk.CTkLabel(c1,
                     text="序列栏：{ } 内为一个键，其余每个字符为一个键名（如 {ctrl}a）。空格需写成 {space}。每条可单独设超时，留空用公共超时。",
                     font=("Microsoft YaHei", 10), text_color=_THEME["text_light"],
                     wraplength=900, justify="left").pack(anchor="w", padx=14, pady=(0, 4))

        inner = ctk.CTkFrame(c1, fg_color="transparent")
        inner.pack(fill="both", expand=True, padx=12, pady=(4, 8))
        self._build_sequences_editable_table(inner)

        # 用 cfg 数据填充序列表格
        self._populate_sequences_table()

    def _parse_leader_seq(self, raw):
        """把序列字符串解析为键段列表。
        规则：整段 {...} 算一个键；其余每个字符算一个键名（空格保留以便标红）。
        返回 (segments, had_space)：segments 为未归一化的原始段（整段 {...} 或单字符）。"""
        segments = []
        had_space = False
        i = 0
        n = len(raw)
        while i < n:
            c = raw[i]
            if c == ' ':
                had_space = True
                segments.append(c)
                i += 1
                continue
            if c == '{':
                j = raw.find('}', i + 1)
                if j == -1:
                    # 未闭合：剩余整体作为一段（标红）
                    segments.append(raw[i:])
                    break
                segments.append(raw[i:j + 1])
                i = j + 1
            else:
                segments.append(c)
                i += 1
        return segments, had_space

    # ──────────────────────────────────────────────
    # 可编辑序列表格（列：超时 | 序列 | 输出 | 删除）
    # 表头与数据行在同一个 grid 内，保证列对齐
    # ──────────────────────────────────────────────
    def _build_sequences_editable_table(self, parent):
        self._seq_rows = []
        self._seq_sentinel_tids = []

        wrapper = ctk.CTkFrame(parent, fg_color="transparent")
        wrapper.pack(fill="both", expand=True, padx=4, pady=(0, 4))

        hdr_frame = ctk.CTkFrame(wrapper, fg_color="transparent")
        hdr_frame.pack(fill="x", pady=(0, 0))

        canvas = tk.Canvas(wrapper, bg=_THEME["card"], highlightthickness=0, relief="flat")
        vsb = _CapsuleScrollbar(wrapper, orient="vertical", command=canvas.yview,
                                bg_color=_THEME["card"], slider_color="#888888",
                                track_width=3, slider_thickness=8)
        canvas.configure(yscrollcommand=lambda *a: (vsb.set(*a), vsb._on_scroll()))
        canvas.pack(side="left", fill="both", expand=True)
        vsb.pack(side="right", fill="y")

        self._seq_canvas = canvas

        inner = ctk.CTkFrame(canvas, fg_color="transparent")
        inner_win = canvas.create_window((0, 0), window=inner, anchor="nw", tags="inner")
        inner.bind("<Configure>", lambda e, c=canvas: c.configure(scrollregion=c.bbox("all")))
        canvas.bind("<Configure>", lambda e, c=canvas: c.itemconfig("inner", width=e.width - 2))
        self._seq_inner = inner
        self._seq_table_inner = inner

        def _on_wheel(e):
            canvas.yview_scroll(-1 if e.delta > 0 else 1, "units")
        canvas.bind("<MouseWheel>", _on_wheel)
        inner.bind("<MouseWheel>", _on_wheel)

        col_weights = [1, 3, 3, 0]
        col_minsize = [70, 120, 160, 32]
        for ci, (w, m) in enumerate(zip(col_weights, col_minsize)):
            hdr_frame.grid_columnconfigure(ci, weight=w, minsize=m)
            inner.grid_columnconfigure(ci, weight=w, minsize=m)

        hdr_txts = ["超时(ms)", "按键序列", "输出", ""]
        for ci, txt in enumerate(hdr_txts):
            lbl = ctk.CTkLabel(hdr_frame, text=txt, font=("Microsoft YaHei", 11, "bold"),
                               text_color=_THEME["text_mid"])
            lbl.grid(row=0, column=ci, sticky="ew", padx=4, pady=(0, 4))

        sep = ctk.CTkFrame(hdr_frame, height=1, fg_color=_THEME["border"])
        sep.grid(row=1, column=0, columnspan=4, sticky="ew", padx=4, pady=(0, 2))

        def _sync_hdr_width(e):
            hdr_frame.configure(width=e.width)
            hdr_frame.pack_configure()
        canvas.bind("<Configure>", _sync_hdr_width, add="+")

        self._seq_next_grid_row = 0
        self._seq_row_widgets = []

    def _populate_sequences_table(self):
        """用 self.cfg 数据填充序列可编辑表格"""
        for rd in self._seq_row_widgets:
            for w in rd["widgets"]:
                try:
                    w.destroy()
                except Exception:
                    pass
        self._seq_rows.clear()
        self._seq_row_widgets.clear()
        self._seq_next_grid_row = 0  # 数据行从 row=0 开始

        for row in self.cfg.get("leader", {}).get("sequences", []):
            # 还原显示串：优先用 _rawSeq（用户原始输入），兼容旧 config 回退拼接 keys
            seq_raw = row.get("_rawSeq", "") or "".join(row.get("keys", []))
            # 超时显示：0 表示未设置，留空更干净
            tm = row.get("timeoutMs", 0)
            self._append_sequence_row(
                str(tm) if tm else "",
                seq_raw,
                row.get("output", ""),
            )
        # 末尾空白哨兵行
        self._append_sequence_sentinel()

    def _validate_sequence_row(self, row_data):
        """校验单行序列：空格 / 键非法 / 键数<2 → 序列格标红。
        非法的行在 _collect 时仍照常保存（运行期匹配不到即安全，见 Q2）。
        空行不标红（清空输入后恢复）。"""
        if row_data["is_sentinel"]:
            return
        t_v, s_v, o_v = row_data["vars"]
        e_seq = row_data["seq_entry"]
        raw_seq = s_v.get()
        if not raw_seq.strip():
            e_seq.configure(text_color=_THEME["text_dark"], border_color=_THEME["border"])
            return
        segments, had_space = self._parse_leader_seq(raw_seq)
        keys = [normalize_key_output(seg) for seg in segments]
        bad = had_space or len(keys) < 2
        for k in keys:
            if not _is_single_key(k):
                bad = True
                break
        if bad:
            e_seq.configure(text_color=_THEME["red"], border_color=_THEME["red"])
        else:
            e_seq.configure(text_color=_THEME["text_dark"], border_color=_THEME["border"])

    def _seq_space_tip(self, row_data):
        """序列格失焦时若含空格，弹提示（Q3：空格需写成 {space}）"""
        t_v, s_v, o_v = row_data["vars"]
        if " " in s_v.get():
            messagebox.showwarning("按键序列",
                                   "序列中包含空格，需写成 {space}（例如 {ctrl}{space}a）",
                                   parent=self)

    def _append_sequence_row(self, timeout="", seq="", output="", is_sentinel=False):
        """追加一行到序列表格，返回创建的 var tuple"""
        parent = self._seq_table_inner
        row_idx = self._seq_next_grid_row
        self._seq_next_grid_row += 1

        timeout_var = ctk.StringVar(value=timeout)
        seq_var = ctk.StringVar(value=seq)
        output_var = ctk.StringVar(value=output)

        entry_kw = dict(
            fg_color=_THEME["card"], border_color=_THEME["border"],
            border_width=1, corner_radius=6, height=32,
            font=("Microsoft YaHei", 11)
        )

        e_timeout = ctk.CTkEntry(parent, textvariable=timeout_var, width=70,
                                 justify="center", **entry_kw)
        e_seq = ctk.CTkEntry(parent, textvariable=seq_var, **entry_kw)
        e_output = ctk.CTkEntry(parent, textvariable=output_var, **entry_kw)

        e_timeout.grid(row=row_idx, column=0, sticky="ew", padx=4, pady=4)
        e_seq.grid(row=row_idx, column=1, sticky="ew", padx=4, pady=4)
        e_output.grid(row=row_idx, column=2, sticky="ew", padx=4, pady=4)

        canvas = self._seq_canvas
        def _entry_wheel(e, c=canvas):
            c.yview_scroll(-1 if e.delta > 0 else 1, "units")
        for entry in [e_timeout, e_seq, e_output]:
            entry.bind("<MouseWheel>", _entry_wheel)

        # 双击弹出大窗口编辑
        for entry, var in [(e_timeout, timeout_var), (e_seq, seq_var),
                            (e_output, output_var)]:
            entry.bind("<Double-Button-1>",
                        lambda e, v=var: self._popup_large_input(v))

        # 自动保存 + 实时校验 + 覆盖/继承颜色框增量刷新
        def _on_change(*_):
            self._validate_sequence_row(row_data)
            self._apply_badge_to_leader_row(row_data)
            self._schedule_autosave()
        for var in (timeout_var, seq_var, output_var):
            var.trace_add("write", _on_change)

        # 序列格失焦：空格弹提示
        # row_data 在第 3386 行才赋值，这里用延迟绑定（函数体内引用），
        # 事件触发时函数早已返回、row_data 已就绪且被闭包持有，避免 UnboundLocalError。
        e_seq.bind("<FocusOut>", lambda e: self._seq_space_tip(row_data))

        # 删除按钮（哨兵行不显示，填写后变为普通行再加）
        del_btn = ctk.CTkButton(parent, text="✕",
                                 fg_color="transparent",
                                 hover_color=_THEME["red"],
                                 text_color=_THEME["text_light"],
                                 corner_radius=4,
                                 width=28, height=28,
                                 font=("Microsoft YaHei", 10, "bold"))
        del_btn.grid(row=row_idx, column=3, padx=4, pady=2)

        vars_tuple = (timeout_var, seq_var, output_var)
        row_data = {
            "vars": vars_tuple,
            "widgets": (e_timeout, e_seq, e_output, del_btn),
            "seq_entry": e_seq,
            "is_sentinel": is_sentinel,
        }

        if is_sentinel:
            del_btn.configure(state="disabled", text="")
        else:
            del_btn.configure(command=lambda rd=row_data: self._del_sequence_row(rd))
            self._validate_sequence_row(row_data)  # 填充已有数据时立即着色

        self._seq_rows.append(vars_tuple)
        self._seq_row_widgets.append(row_data)
        return vars_tuple

    def _append_sequence_sentinel(self):
        """在末尾追加哨兵行，监听输入后自动变为普通行"""
        vars_tuple = self._append_sequence_row(is_sentinel=True)
        timeout_var, seq_var, output_var = vars_tuple

        tids = []

        def _on_change(*_):
            if any(v.get().strip() for v in (timeout_var, seq_var, output_var)):
                for var, tid in zip((timeout_var, seq_var, output_var), tids):
                    try:
                        var.trace_remove("write", tid)
                    except Exception:
                        pass
                for rd in self._seq_row_widgets:
                    if rd["vars"] is vars_tuple and rd["is_sentinel"]:
                        rd["is_sentinel"] = False
                        del_btn = rd["widgets"][3]
                        del_btn.configure(state="normal", text="✕",
                                          command=lambda r=rd: self._del_sequence_row(r))
                        break
                self._append_sequence_sentinel()

        for var in (timeout_var, seq_var, output_var):
            tid = var.trace_add("write", _on_change)
            tids.append(tid)

    def _del_sequence_row(self, row_data):
        """删除一行序列"""
        for w in row_data["widgets"]:
            if isinstance(w, tk.Widget):
                try:
                    w.destroy()
                except Exception:
                    pass
        if row_data in self._seq_row_widgets:
            self._seq_row_widgets.remove(row_data)
        if row_data["vars"] in self._seq_rows:
            self._seq_rows.remove(row_data["vars"])
        self._schedule_autosave()

    # ── 收集序列表格数据 ────────────────────────
    def _collect_leader_rows(self):
        """从 _seq_row_widgets 收集非空非哨兵的行。
        非法键（含空格）仍保存该行（运行期匹配不到即安全，见 Q2）。"""
        result = []
        for rd in self._seq_row_widgets:
            if rd["is_sentinel"]:
                continue
            t_v, s_v, o_v = rd["vars"]
            raw_seq = s_v.get().strip()
            raw_out = o_v.get()
            if not raw_seq or not raw_out.strip():
                continue  # 不完整行跳过
            segments, _ = self._parse_leader_seq(raw_seq)
            keys = [normalize_key_output(seg) for seg in segments]
            out = normalize_key_output(raw_out)
            try:
                t = int(t_v.get().strip())
            except (ValueError, AttributeError):
                t = 0
            result.append({
                "keys": keys,
                "output": out,
                "timeoutMs": t,
                "_rawSeq": raw_seq,   # 保存原始输入用于 GUI 显示（不参与引擎匹配）
            })
        return result

    # ── 收集数据 ───────────────────────────
    # ── per-device 映射覆盖：活动映射交换 ──
    # 设计：self._config_master = 权威配置（全局映射段 + devices 桶）；
    #       self.cfg 的 comboMap/layers/leader/tapDance 段 = 当前编辑目标的「解析视图」。
    # 切目标：_flush_active_mapping() 先把当前控件值收回 master →
    #       把 self.cfg 映射段指向目标视图 → 重载编辑器。
    # flush：当前控件值 与 master 全局 按 identity 做 diff，只把被改条目写进 devices[guid]
    #       （全局模式则写回 master 顶层）。TD 绑定走 layers.baseLayer 的 KeyEntry（7 字段），
    #       自动被 layers 段覆盖，无需单独桶。设备设置不向上同步全局。

    @staticmethod
    def _dc(x):
        import json as _j
        return _j.loads(_j.dumps(x, ensure_ascii=False)) if x is not None else None

    @staticmethod
    def _device_edit_key(guid, vid, pid, dtype):
        """设备覆盖桶键：与引擎 build_multi_device_contexts 一致。
        有 guid 用 guid；否则用 VID:PID:type（VID/PID 大写，type 小写）。"""
        if guid:
            return guid
        v = str(vid or "").upper()
        p = str(pid or "").upper()
        t = str(dtype or "keyboard").lower()
        if not v or not p:
            return None
        return f"{v}:{p}:{t}"

    def _combo_ident(self, row):
        # 注意：combo 的 identity 必须包含 layer！同一对按键可同时存在于
        # base / fn2 等多层（如 a+s→{Left}@base 与 a+s→{home}@fn2）。
        # 若只用 key1+key2，_diff_combos / _merge_combos 的 by 字典会发生
        # 键冲突，把另一层的同名组合覆盖掉，导致「无覆盖设备」被误判为有
        # 设备专属覆盖，从而把全局 combo 写进 devices[guid]（见用户反馈）。
        ks = sorted(_norm_key(k).lower() for k in (row.get("key1", ""), row.get("key2", "")) if str(k).strip())
        layer = (row.get("layer") or "base") or "base"
        return f"{layer}:" + "+".join(ks)

    def _leader_ident(self, seq):
        return ",".join(_norm_key(k).lower() for k in seq.get("keys", []) if str(k).strip())

    @staticmethod
    def _flatten_layers(layers):
        """扁平化 tapDance: {layerName: {key: entry}} → {(layerName, key): entry}"""
        out = {}
        if not isinstance(layers, dict):
            return out
        for ln, km in layers.items():
            if isinstance(km, dict) and ln not in ("holdTerm", "doubleTapTerm", "doubleHoldTerm"):
                for k, e in km.items():
                    out[(ln, k)] = e
        return out

    @staticmethod
    @staticmethod
    def _row_differs(a, b):
        import json as _j
        return _j.dumps(a, sort_keys=True, ensure_ascii=False) != _j.dumps(b, sort_keys=True, ensure_ascii=False)

    @staticmethod
    def _norm_entry(entry):
        """归一化单个 KeyEntry 的输出字段（tap/hold/dt/dh 裸键名→{X}），返回副本。
        用于 diff 前对齐全局与设备视图，避免归一化差异造成的假覆盖。"""
        if not isinstance(entry, dict):
            return entry
        e = dict(entry)
        for f in ("tap", "hold", "dt", "dh"):
            if e.get(f):
                try:
                    e[f] = normalize_key_output(e[f])
                except Exception:
                    pass
        return e

    def _merge_combos(self, g, ov):
        if not ov:
            return [dict(r) for r in (g or [])]
        by = {}
        for r in (g or []):
            by[self._combo_ident(r)] = dict(r)
        for r in ov:
            by[self._combo_ident(r)] = dict(r)
        return list(by.values())

    def _merge_leader(self, g, ov_seqs):
        merged = dict(g) if isinstance(g, dict) else {"sequences": []}
        gseqs = (g or {}).get("sequences", [])
        if not ov_seqs:
            return merged
        by = {}
        for s in gseqs:
            by[self._leader_ident(s)] = dict(s)
        for s in ov_seqs:
            by[self._leader_ident(s)] = dict(s)
        merged["sequences"] = list(by.values())
        return merged

    def _merge_layers(self, g, ov):
        """合并两层 tapDance。g 和 ov 均为扁平格式 {layerName: {key: entry}}。"""
        if not ov:
            return self._dc(g) or {}
        out = self._dc(g) or {}
        for ln, km in (ov or {}).items():
            if isinstance(km, dict):
                merged = dict(out.get(ln) or {})
                for k, e in km.items():
                    merged[k] = dict(e)
                out[ln] = merged
        return out

    def _diff_combos(self, g, view):
        by = {}
        for r in (g or []):
            by[self._combo_ident(r)] = r
        out = []
        for r in (view or []):
            ident = self._combo_ident(r)
            gv = by.get(ident)
            if gv is None or self._row_differs(gv, r):
                out.append(dict(r))
        return out

    def _diff_leader(self, g, view):
        gby = {}
        for s in (g or {}).get("sequences", []):
            gby[self._leader_ident(s)] = s
        out = []
        for s in (view or {}).get("sequences", []):
            ident = self._leader_ident(s)
            gv = gby.get(ident)
            if gv is None or self._row_differs(gv, s):
                out.append(dict(s))
        return out

    def _diff_layers(self, g, view):
        """返回全格式 diff：{baseLayer: {...}, layerDefs: [{name, keyMap}]}，与全局 tapDance 格式一致。"""
        gflat = self._flatten_layers(g or {})
        vflat = self._flatten_layers(view or {})
        out_base = {}
        out_defs = {}  # {layerName: {key: entry}}
        for (ln, k), e in vflat.items():
            en = self._norm_entry(e)
            ge = gflat.get((ln, k))
            gen = self._norm_entry(ge) if ge is not None else None
            if gen is None or self._row_differs(gen, en):
                if ln == "base":
                    out_base[k] = en
                else:
                    out_defs.setdefault(ln, {})[k] = en
        result = {}
        if out_base:
            result["base"] = out_base
        for nm, km in out_defs.items():
            result[nm] = km
        return result

    def _resolve_device_view(self, guid):
        g = self._config_master
        g_combo = g.get("comboMap", [])
        g_layers = g.get("tapDance", dict(EMPTY_LAYERS))
        g_leader = g.get("leader", dict(EMPTY_LEADER))
        ov = (g.get("devices") or {}).get(guid)
        if not ov:
            return (self._dc(g_combo), self._dc(g_layers), self._dc(g_leader))
        return (self._merge_combos(g_combo, ov.get("comboMap")),
                self._merge_layers(g_layers, ov.get("tapDance")),
                self._merge_leader(g_leader, ov.get("leader")))

    def _resolve_app_view(self, guid, app):
        """四层合并视图：全局 → appAware → devices[guid] → devices[guid].apps[app]
        返回旧格式，供编辑器直接使用。"""
        g = self._config_master
        combo = self._dc(g.get("comboMap", []))
        layers = self._dc(g.get("tapDance", dict(EMPTY_LAYERS)))
        leader = self._dc(g.get("leader", dict(EMPTY_LEADER)))

        # Layer 1: appAware.apps[app]
        if app:
            app_ov = (g.get("appAware") or {}).get("apps", {}).get(app, {})
            combo = self._merge_combos(combo, app_ov.get("comboMap"))
            layers = self._merge_layers(layers, app_ov.get("tapDance"))
            leader = self._merge_leader(leader, app_ov.get("leader"))

        # Layer 2: devices[guid]
        if guid:
            dev_ov = (g.get("devices") or {}).get(guid, {})
            combo = self._merge_combos(combo, dev_ov.get("comboMap"))
            layers = self._merge_layers(layers, dev_ov.get("tapDance"))
            leader = self._merge_leader(leader, dev_ov.get("leader"))

            # Layer 3: devices[guid].apps[app]
            if app:
                dev_app_ov = (dev_ov.get("apps") or {}).get(app, {})
                combo = self._merge_combos(combo, dev_app_ov.get("comboMap"))
                layers = self._merge_layers(layers, dev_app_ov.get("tapDance"))
                leader = self._merge_leader(leader, dev_app_ov.get("leader"))

        return combo, layers, leader

    def _resolve_baseline(self, device, app):
        """解析到当前 scope 的上一层（不含最内层覆盖桶）。
        device+app → 全局 + appAware + 设备（不含 devices[tgt].apps[app]）
        全局+app   → 全局（不含 appAware）
        设备+全局  → 全局（不含 devices[tgt]）
        全局+全局  → 空（无需基线）"""
        if device and app:
            # 需要全局 + appAware + devices，但不走 Layer 3
            g = self._config_master
            combo = self._dc(g.get("comboMap", []))
            layers = self._dc(g.get("tapDance", dict(EMPTY_LAYERS)))
            leader = self._dc(g.get("leader", dict(EMPTY_LEADER)))
            # Layer 1: appAware.apps[app]
            app_ov = (g.get("appAware") or {}).get("apps", {}).get(app, {})
            combo = self._merge_combos(combo, app_ov.get("comboMap"))
            layers = self._merge_layers(layers, app_ov.get("tapDance"))
            leader = self._merge_leader(leader, app_ov.get("leader"))
            # Layer 2: devices[device]（不含 devices[device].apps[app]）
            dev_ov = (g.get("devices") or {}).get(device, {})
            combo = self._merge_combos(combo, dev_ov.get("comboMap"))
            layers = self._merge_layers(layers, dev_ov.get("tapDance"))
            leader = self._merge_leader(leader, dev_ov.get("leader"))
            return combo, layers, leader
        elif app:
            # 全局+app：基线 = 全局
            return self._resolve_app_view(None, "")
        elif device:
            # 设备+全局：基线 = 全局
            g = self._config_master
            return (self._dc(g.get("comboMap", [])),
                    self._dc(g.get("tapDance", dict(EMPTY_LAYERS))),
                    self._dc(g.get("leader", dict(EMPTY_LEADER))))
        else:
            return None  # 全局+全局，无需 diff

    def _get_edit_container(self, device, app):
        """按 (设备, 应用) 定位写回容器"""
        master = self._config_master
        if device and app:
            return master.setdefault("devices", {}).setdefault(device, {}).setdefault("apps", {}).setdefault(app, {})
        elif device:
            return master.setdefault("devices", {}).setdefault(device, {})
        elif app:
            return master.setdefault("appAware", {}).setdefault("apps", {}).setdefault(app, {})
        else:
            return master  # 全局

    def _flush_active_mapping(self):
        try:
            master = getattr(self, "_config_master", None) or self.cfg
            tgt = getattr(self, "_edit_target", None)
            app = getattr(self, "_current_app", "")

            combo_tab_built = hasattr(self, "_combo_table_inner")
            layer_tab_built = hasattr(self, "_layer_tab_frame")
            seq_tab_built  = hasattr(self, "_seq_table_inner")
            view_combo = self._collect_combo_rows() if hasattr(self, "_combo_row_widgets") else self.cfg.get("comboMap", [])
            view_layers = self._collect_layer_cfg() if (getattr(self, "_switch_key_vars", None) and getattr(self, "_layer_key_trees", None)) else self.cfg.get("tapDance", {})
            view_leader = self._collect_leader_rows() if seq_tab_built else self._dc(self.cfg.get("leader", {}))

            # 归一化 layers 输出（就地）
            _vl = view_layers
            if isinstance(_vl, dict):
                _norm = {"tapDance": _vl}
                normalize_layers_key_outputs(_norm)
            # leader 转为 dict
            if not isinstance(view_leader, dict):
                _vl_leader = dict(master.get("leader", dict(EMPTY_LEADER)))
                _vl_leader["sequences"] = view_leader
                view_leader = _vl_leader

            # 至少有一个 tab 构建过才 flush（无构建时说明刚启动，保护 master 不被清空）
            if not (combo_tab_built or layer_tab_built or seq_tab_built):
                return

            if app:
                # 应用感知：diff vs 不含当前 scope 覆盖桶的基线
                base_combo, base_layers, base_leader = self._resolve_baseline(tgt, app)
                # 归一化 base layers（就地）
                if isinstance(base_layers, dict):
                    normalize_layers_key_outputs({"tapDance": base_layers})
                d_combo = self._diff_combos(base_combo, view_combo)
                d_layers = self._diff_layers(base_layers, _vl)
                d_leader = self._diff_leader(base_leader, view_leader)
                container = self._get_edit_container(tgt, app)
                # 清除旧字段
                for k in ("comboMap", "tapDance", "leader"):
                    container.pop(k, None)
                if d_combo:
                    container["comboMap"] = d_combo
                if d_layers:
                    container["tapDance"] = d_layers
                if d_leader:
                    container["leader"] = d_leader
                # 容器为空则删
                if not container:
                    self._remove_edit_container(tgt, app)
            elif tgt is not None:
                # 设备覆盖（无 app）：现有 diff 逻辑
                g_combo = master.get("comboMap", [])
                g_layers = master.get("tapDance", dict(EMPTY_LAYERS))
                g_leader = master.get("leader", dict(EMPTY_LEADER))
                if not isinstance(view_leader, dict):
                    _vl2 = dict(g_leader)
                    _vl2["sequences"] = view_leader
                    view_leader = _vl2
                d_combo = self._diff_combos(g_combo, view_combo)
                d_layers = self._diff_layers(g_layers, _vl)
                d_leader = self._diff_leader(g_leader, view_leader)
                bucket = {}
                if d_combo:
                    bucket["comboMap"] = d_combo
                if d_layers:
                    bucket["tapDance"] = d_layers
                if d_leader:
                    bucket["leader"] = d_leader
                devices = master.setdefault("devices", {})
                if bucket:
                    # 合并而非替换，保留已有的 apps 等子桶
                    existing = dict(devices.get(tgt) or {})
                    existing.update(bucket)
                    devices[tgt] = existing
                else:
                    devices.pop(tgt, None)
                master["devices"] = {k: v for k, v in devices.items() if v}
            else:
                # 全局编辑：直接写回 master 顶层
                master["comboMap"] = view_combo
                master["tapDance"] = _vl
                master["leader"] = view_leader
            # 同步 self.cfg（转为旧格式供编辑器使用）
            self.cfg["comboMap"] = self._dc(view_combo)
            self.cfg["tapDance"] = self._dc(_vl)
            self.cfg["leader"] = self._dc(view_leader)
        except Exception as e:
            print("flush_active_mapping error:", e)

    def _remove_edit_container(self, device, app):
        """清空后若容器为空则逐层清理"""
        master = self._config_master
        if device and app:
            dd = master.get("devices", {}).get(device, {})
            da = dd.get("apps", {})
            if app in da and not da[app]:
                del da[app]
            if not da:
                dd.pop("apps", None)
            if not dd:
                master.setdefault("devices", {}).pop(device, None)
        elif device:
            master.setdefault("devices", {}).pop(device, None)
        elif app:
            a = master.get("appAware", {}).get("apps", {})
            if app in a and not a[app]:
                del a[app]

    def _select_edit_target(self, guid):
        try:
            self._flush_active_mapping()
            self._edit_target = guid
            app = getattr(self, "_current_app", "")
            combo, layers, leader = self._resolve_app_view(guid, app)
            self.cfg["comboMap"] = combo
            self.cfg["tapDance"] = layers
            self.cfg["leader"] = leader
            self._reload_editors()
            self._update_device_selection_ui()
            self._update_edit_target_banner()
        except Exception as e:
            print("select_edit_target error:", e)

    def _reload_editors(self):
        # 标签页是懒构建（switch_tab 首次显示时才建），未构建的标签其控件属性
        # （_seq_row_widgets/_seq_inner/_layer_* 等）尚不存在；直接调 populate 会触发
        # CTk __getattr__ 把缺失属性转发到 tkapp，报 "object has no attribute '...'"。
        # 故只重载已构建（已显示过、在 _tab_frames 中）的标签；未构建的标签
        # 在首次 switch_tab 时已自带填充，无需在此处理。
        _built = getattr(self, "_tab_frames", {}) or {}
        _tab_for = {"_populate_combo_table": "combo",
                    "_reload_layer_cards": "layer",
                    "_populate_sequences_table": "sequences"}
        for fn, tab in _tab_for.items():
            if tab not in _built:
                continue
            f = getattr(self, fn, None)
            if callable(f):
                try:
                    f()
                except Exception as e:
                    print("reload", fn, "error:", e)
        # TD 计时变量始终全局（per-device 计时保持全局）
        try:
            td = self._config_master.get("tapDance", {}) if hasattr(self, "_config_master") else self.cfg.get("tapDance", {})
            if hasattr(self, "_td_ht_var"):
                self._td_ht_var.set(str(td.get("holdTerm", 200)))
            if hasattr(self, "_td_dt_var"):
                self._td_dt_var.set(str(td.get("doubleTapTerm", 250)))
            if hasattr(self, "_td_dh_var"):
                self._td_dh_var.set(str(td.get("doubleHoldTerm", 200)))
        except Exception:
            pass
        try:
            self._apply_override_badges()
        except Exception as e:
            print("apply_override_badges error:", e)

    def _owned_identities(self, guid):
        ov = (self._config_master.get("devices") or {}).get(guid) or {}
        s = set()
        for r in (ov.get("comboMap") or []):
            s.add(("combo", self._combo_ident(r)))
        for seq in (ov.get("leader") or []):
            s.add(("leader", self._leader_ident(seq)))
        # tapDance 扁平格式
        flat = self._flatten_layers(ov.get("tapDance"))
        for (ln, k) in flat:
            s.add(("layers", ln, k))
        return s

    def _global_identities(self):
        """返回全局配置中所有条目的身份集合（不是覆盖桶，是顶层全局段）。"""
        g = self._config_master
        s = set()
        for row in (g.get("comboMap") or []):
            s.add(("combo", self._combo_ident(row)))
        for seq in (g.get("leader", {}).get("sequences") or []):
            s.add(("leader", self._leader_ident(seq)))
        # 全局 layers 是完整结构，flatten 后 key=(layerName, physKey)
        g_layers = g.get("tapDance", {})
        flat = self._flatten_layers(g_layers)
        for (ln, k), entry in flat.items():
            s.add(("layers", ln, k))
        return s

    def _app_override_identities(self, tgt, app):
        """返回当前 app 覆盖桶中所有条目的身份集合。"""
        master = self._config_master
        s = set()
        ov = {}
        if tgt and app:
            ov = ((master.get("devices") or {}).get(tgt, {}).get("apps", {}) or {}).get(app, {})
        elif app:
            ov = (master.get("appAware") or {}).get("apps", {}).get(app, {})
        if not ov:
            return s
        for r in (ov.get("comboMap") or []):
            s.add(("combo", self._combo_ident(r)))
        for seq in (ov.get("leader") or []):
            s.add(("leader", self._leader_ident(seq)))
        flat = self._flatten_layers(ov.get("tapDance"))
        for (ln, k) in flat:
            s.add(("layers", ln, k))
        return s

    def _update_device_selection_ui(self):
        tgt = getattr(self, "_edit_target", None)
        if getattr(self, "_global_entry_row", None) is not None:
            gr = self._global_entry_row
            sel = tgt is None
            try:
                # 全局设置选中时只加粗蓝色边框，背景保持 card 不变；
                # 未选中 1px、选中 3px，与下方设备行完全一致。
                gr.configure(border_color=_THEME["blue"] if sel else _THEME["border"],
                             border_width=3 if sel else 1,
                             fg_color=_THEME["card"])
                # 蓝色边框提到最上层，避免被相邻设备卡/滚动条视觉压住
                try: gr.tkraise()
                except Exception: pass
            except Exception:
                pass
            # 图标：选中时变主题蓝，未选中恢复深色。
            icon = getattr(self, "_global_entry_icon", None)
            if icon is not None:
                try:
                    icon.configure(text_color=_THEME["blue"] if sel else _THEME["text_dark"])
                except Exception:
                    pass
        for row in getattr(self, "_device_rows", []):
            rframe = row.get("frame")
            if rframe is None:
                continue
            sel = (row.get("_edit_key") is not None) and (row.get("_edit_key") == tgt) and (tgt is not None)
            try:
                rframe.configure(
                    border_color=_THEME["blue"] if sel else _THEME["border"],
                    border_width=3 if sel else 1)
            except Exception:
                pass

    def _short_device_label(self, tgt):
        """设备短标签：优先别名，其次 VID:PID，最后才回退原始键。

        状态栏用途：GUID/完整设备标识太长且无识别性，别名未设置时
        显示 VID:PID（如 046D:C539）即可区分设备。"""
        if not tgt:
            return tgt
        aliases = self.cfg.get("device_aliases") or {}
        # 1) 别名直接命中（兼容保存时的大小写差异）
        if tgt in aliases:
            return aliases[tgt]
        if tgt.upper() in aliases:
            return aliases[tgt.upper()]
        # 2) 在设备行中定位该键，取 VID:PID；顺带再试 VID:PID:TYPE 形式的别名键
        for row in getattr(self, "_device_rows", []):
            if row.get("_edit_key") != tgt:
                continue
            vid = row.get("vid") or ""
            pid = row.get("pid") or ""
            if vid and pid:
                vp = f"{vid}:{pid}".upper()
                if vp in aliases:
                    return aliases[vp]
                if f"{vp}:{str(row.get('type') or 'keyboard').lower()}".upper() in aliases:
                    return aliases[f"{vp}:{str(row.get('type') or 'keyboard').lower()}".upper()]
                return vp
            break
        return tgt

    def _update_edit_target_banner(self):
        banner = getattr(self, "_edit_banner", None)
        if not banner:
            return
        tgt = getattr(self, "_edit_target", None)
        app = getattr(self, "_current_app", "")
        
        # 构建横幅文本
        if tgt is None and not app:
            banner.configure(text="全局基础层（所有设备默认继承）", text_color=_THEME["text_light"])
        elif tgt is None:
            banner.configure(text=f"应用: {app}", text_color=_THEME["override"])
        else:
            alias = self._short_device_label(tgt)
            owned = self._owned_identities(tgt)
            glob = self._global_identities()
            overrides = owned & glob
            supplements = owned - glob
            n_ov = len(overrides)
            n_sup = len(supplements)
            if n_ov:
                tag = f"{n_ov} 条覆盖" + (f" + {n_sup} 条补充" if n_sup else "")
            else:
                tag = f"完全继承全局" + (f"（{n_sup} 条补充）" if n_sup else "")
            base = f"设备『{alias}』— {tag}"
            if app:
                banner.configure(text=f"{base} | 应用: {app}", text_color=_THEME["override"])
            else:
                banner.configure(text=base, text_color=(_THEME["override"] if n_ov else _THEME["text_light"]))
        
        # 刷新应用下拉（同步圆点标记）
        if hasattr(self, '_app_panel'):
            self._refresh_app_dropdown()

    # ── 应用感知回调 ──
    def _on_app_selected(self, app: str):
        # 先以旧 app 身份 flush 当前编辑，再切 scope
        self._flush_active_mapping()
        self._current_app = app
        # 加载新 scope 的视图（不复 flush，避免污染全局/设备）
        tgt = self._edit_target
        combo, layers, leader = self._resolve_app_view(tgt, app)
        self.cfg["comboMap"] = combo
        self.cfg["tapDance"] = layers
        self.cfg["leader"] = leader
        self._reload_editors()
        self._update_edit_target_banner()
        # app 覆盖增删后，cfg["appAware"] 可能变化 → 同步下拉圆点
        self._refresh_app_dropdown()

    def _refresh_app_dropdown(self):
        """从最新 cfg 同步 AppPanel._configured（含主 appAware + 设备级 apps）。
        让下拉圆点（● 已配置 / ○ 仅运行）随配置变化即时更新。"""
        if not hasattr(self, '_app_panel'):
            return
        configured = set((self.cfg.get("appAware") or {}).get("apps", {}).keys())
        for dev_cfg in (self.cfg.get("devices") or {}).values():
            if isinstance(dev_cfg, dict):
                configured.update((dev_cfg.get("apps") or {}).keys())
        self._app_panel.set_configured_apps(list(configured))
        self._app_panel.set_selected(self._current_app)

    def _on_delete_page_clicked(self):
        label = self._current_app or "全局"
        if not messagebox.askyesno(
                "删除当前页设置",
                f"确定清空 [{label}] 当前页面的设置吗？\n"
                f"（如需清空全部设置，请在 Combo / TapDance / Leader 三页分别删除）\n"
                "该操作不可撤销。", parent=self):
            return
        master = self._config_master
        page = self._current_tab
        device = self._edit_target
        app = self._current_app
        container = self._get_edit_container(device, app)
        self._clear_page_in_override(container, page)
        if all(not (k in container and container[k]) for k in ("comboMap", "tapDance", "leader")):
            self._remove_edit_container(device, app)
        self._schedule_autosave()
        self._reload_view()

    def _export_app_page(self):
        page = self._current_tab
        data = self._collect_page_data(page)
        if not data:
            messagebox.showinfo("提示", "当前覆盖层无数据。")
            return
        path = filedialog.asksaveasfilename(defaultextension=".json", filetypes=[("JSON", "*.json")])
        if not path:
            return
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
        messagebox.showinfo("导出成功", f"已导出到 {os.path.basename(path)}")

    def _import_app_page(self):
        path = filedialog.askopenfilename(filetypes=[("JSON", "*.json")])
        if not path:
            return
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except Exception as e:
            messagebox.showerror("错误", f"读取失败: {e}")
            return
        page = self._current_tab
        container = self._get_edit_container(self._edit_target, self._current_app)
        self._apply_page_data_to_container(container, page, data)
        self._schedule_autosave()
        self._reload_view()
        messagebox.showinfo("导入成功", "已导入设置。")

    def _collect_page_data(self, page):
        container = self._get_edit_container(self._edit_target, self._current_app)
        out = {}
        if page == "combo" and container.get("comboMap"):
            out["comboMap"] = container["comboMap"]
            if "comboTime" in container:
                out["comboTime"] = container["comboTime"]
        elif page == "layer" and (container.get("tapDance") or container.get("layers")):
            out["tapDance"] = container.get("tapDance") or container.get("layers")
        elif page == "sequences" and container.get("leader"):
            out["leader"] = container["leader"]
        return out or None

    @staticmethod
    def _apply_page_data_to_container(container, page, data):
        if page == "combo":
            container["comboMap"] = data.get("comboMap", [])
            if "comboTime" in data:
                container["comboTime"] = data["comboTime"]
        elif page == "layer" and "tapDance" in data:
            container["tapDance"] = data["tapDance"]
        elif page == "layer" and "layers" in data:
            container["tapDance"] = data["layers"]
        elif page == "sequences" and "leader" in data:
            container["leader"] = data["leader"]

    @staticmethod
    def _clear_page_in_override(ov, page):
        if page == "combo":
            ov.pop("comboMap", None)
        elif page == "layer":
            ov.pop("tapDance", None)
            ov.pop("layers", None)
        elif page == "sequences":
            ov.pop("leader", None)

    def _reload_view(self):
        tgt = self._edit_target
        app = self._current_app
        combo, layers, leader = self._resolve_app_view(tgt, app)
        self.cfg["comboMap"] = combo
        self.cfg["tapDance"] = layers
        self.cfg["leader"] = leader
        self._reload_editors()
        self._update_edit_target_banner()

    def _reload_current_page(self):
        if self._current_tab:
            self._switch_tab(self._current_tab)
    def _badge_sets(self):
        """返回当前设备编辑目标下的 (overrides, inherited) 集合。
        overrides = 全局存在且设备桶重写的 ident；inherited = 全局存在但设备桶没有的 ident。
        无设备目标时返回空集合。"""
        tgt = getattr(self, "_edit_target", None)
        if tgt is None:
            return set(), set()
        owned = self._owned_identities(tgt)
        glob = self._global_identities()
        return owned & glob, glob - owned

    def _scope_identities(self):
        """返回当前 scope 下的 (owned_set, owned_color, inherit_color)。已弃用，请用 _layer_tag。"""
        tgt = getattr(self, "_edit_target", None)
        app = getattr(self, "_current_app", "")
        if app:
            owned = self._app_override_identities(tgt, app)
            return owned, _THEME["green"], _THEME["inherit"]
        if tgt is not None:
            overrides, _ = self._badge_sets()
            return overrides, _THEME["override"], _THEME["inherit"]
        return set(), _THEME["border"], _THEME["border"]

    # ── 四层四色系统 ──
    _FOUR_COLOR = {
        "global":      ("#4A80BD", "#5A6A80"),  # 蓝（全局基数）
        "device":      ("#3FB68B", "#507068"),  # 绿（设备覆盖）
        "global_app":  ("#C8A030", "#887840"),  # 黄（全局应用覆盖）
        "device_app":  ("#D06830", "#886048"),  # 橙（设备+应用覆盖）
        "new":         ("#888888", "#666666"),  # 灰（新增/未存储）
    }

    def _layer_tag(self, ident):
        """返回 (color_full, color_dim, is_owned, layer_name)。
        is_owned 仅当条目属于【当前 scope 的写入容器】时为 True。
        设备+app → 只有 devices[tgt].apps[app] 是 owned
        全局+app → 只有 appAware.apps[app] 是 owned
        设备+全局 → 只有 devices[tgt] 是 owned
        全局+全局 → 全部是 global，无 owned 标记"""
        tgt = getattr(self, "_edit_target", None)
        app = getattr(self, "_current_app", "")

        if tgt and app:
            # Scope: 设备+应用
            if ident in self._app_override_identities(tgt, app):
                return (*self._FOUR_COLOR["device_app"], True, "device_app")
            if ident in self._app_override_identities(None, app):
                return (*self._FOUR_COLOR["global_app"], False, "global_app")
            if ident in self._owned_identities(tgt):
                return (*self._FOUR_COLOR["device"], False, "device")
            if ident in self._global_identities():
                return (*self._FOUR_COLOR["global"], False, "global")
        elif app:
            # Scope: 全局+应用
            if ident in self._app_override_identities(None, app):
                return (*self._FOUR_COLOR["global_app"], True, "global_app")
            if ident in self._global_identities():
                return (*self._FOUR_COLOR["global"], False, "global")
        elif tgt:
            # Scope: 设备+全局
            if ident in self._owned_identities(tgt):
                return (*self._FOUR_COLOR["device"], True, "device")
            if ident in self._global_identities():
                return (*self._FOUR_COLOR["global"], False, "global")
        else:
            # Scope: 全局+全局 — 所有条目同色
            pass

        return (*self._FOUR_COLOR["new"], False, "new")

    def _badge_border(self, ident, owned_w=2, inherited_w=1):
        """返回 (color, width)。owned → 粗边；继承 → 同色细边。默认 2/1（combo/leader 行）。"""
        full, _, owned, _ = self._layer_tag(ident)
        return (full, owned_w) if owned else (full, inherited_w)

    def _badge_reset_ident(self, ident):
        """返回可右键重置的 ident，仅当条目属于当前 scope 覆盖桶时非 None。"""
        _, _, owned, _ = self._layer_tag(ident)
        return ident if owned else None

    def _apply_badge_to_combo_row(self, rd, owned=None, glob=None):
        """刷新单条 combo 行的四层着色（未传参则用当前 scope）。"""
        if rd.get("is_sentinel"):
            return
        try:
            lv, k1v, k2v, outv = rd["vars"]
            k1 = k1v.get().strip(); k2 = k2v.get().strip()
            layer_raw = lv.get().strip()
            layer = _layer_from_display(layer_raw) if layer_raw else "base"
            if not k1 or not k2:
                for e in rd["widgets"][:4]:
                    e.configure(border_color=_THEME["border"])
                self._bind_row_reset(rd["widgets"], None)
                return
            row = {"key1": k1, "key2": k2, "layer": layer}
            ident = ("combo", self._combo_ident(row))
            color, width = self._badge_border(ident)
            for e in rd["widgets"][:4]:
                e.configure(border_color=color, border_width=width)
            self._bind_row_reset(rd["widgets"], self._badge_reset_ident(ident))
        except Exception:
            pass

    def _apply_badge_to_leader_row(self, rd, owned=None, glob=None):
        """刷新单条 leader 行的四层着色。"""
        if rd.get("is_sentinel"):
            return
        try:
            raw_seq = rd["vars"][1].get().strip()
            segments, _ = self._parse_leader_seq(raw_seq)
            keys = [normalize_key_output(seg) for seg in segments]
            if not keys:
                for e in rd["widgets"][:3]:
                    try: e.configure(border_color=_THEME["border"])
                    except Exception: pass
                self._bind_row_reset(rd["widgets"][:3], None)
                return
            ident = ("leader", self._leader_ident({"keys": keys}))
            color, width = self._badge_border(ident)
            for e in rd["widgets"][:3]:
                try: e.configure(border_color=color, border_width=width)
                except Exception: pass
            self._bind_row_reset(rd["widgets"][:3], self._badge_reset_ident(ident))
        except Exception:
            pass

    def _apply_badge_to_layer_key(self, phys_key, owned=None, glob=None):
        """刷新当前层键在键盘图上的覆盖/继承边框颜色。
        输入框与宏列表保持默认颜色，颜色只画在键盘图按键边框上。"""
        # 重置 7 个输入框边框为默认（不再在 entry 上显示颜色）
        for e in getattr(self, "_kb_entries", []):
            try:
                e.configure(border_color=_THEME["border"])
            except Exception:
                pass
        # 颜色统一在键盘图矩形边框上绘制
        self._refresh_kb_colors()

    def _apply_override_badges(self):
        """统一着色：根据当前 scope 显示 owned/inherited 颜色。"""
        for rd in getattr(self, "_combo_row_widgets", []):
            self._apply_badge_to_combo_row(rd, None, None)
        for rd in getattr(self, "_seq_row_widgets", []):
            self._apply_badge_to_leader_row(rd, None, None)

    def _bind_row_reset(self, widgets, ident):
        """给一行控件绑右键：ident 非空→重置该覆盖为全局；否则解绑。"""
        for w in widgets:
            try:
                if ident is None:
                    w.unbind("<Button-3>")
                else:
                    w.bind("<Button-3>", lambda _e=None, i=ident: self._reset_override_entry(i))
            except Exception:
                pass

    def _reset_override_entry(self, ident):
        """把单条覆盖从当前设备桶移除，恢复继承全局。"""
        tgt = getattr(self, "_edit_target", None)
        if tgt is None:
            return
        devices = self._config_master.setdefault("devices", {})
        bucket = devices.get(tgt)
        if not bucket:
            return
        kind = ident[0]
        if kind == "combo":
            rows = bucket.get("comboMap") or []
            bucket["comboMap"] = [r for r in rows if self._combo_ident(r) != ident[1]]
            if not bucket["comboMap"]:
                bucket.pop("comboMap", None)
        elif kind == "leader":
            seqs = bucket.get("leader") or []
            bucket["leader"] = [s for s in seqs if self._leader_ident(s) != ident[1]]
            if not bucket["leader"]:
                bucket.pop("leader", None)
        elif kind == "layers":
            _, ln, k = ident
            lay = bucket.get("tapDance") or {}
            if ln in lay:
                lay[ln].pop(k, None)
                if not lay[ln]:
                    lay.pop(ln, None)
            bucket["tapDance"] = lay
            if not bucket["tapDance"]:
                bucket.pop("layers", None)
        if not bucket:
            devices.pop(tgt, None)
        # 重新解析视图并重载
        combo, layers, leader = self._resolve_device_view(tgt)
        self.cfg["comboMap"] = combo
        self.cfg["tapDance"] = layers
        self.cfg["leader"] = leader
        self._reload_editors()
        self._update_edit_target_banner()
        self._schedule_autosave()

    def _collect_cfg(self):
        # ── per-device 权威配置流程 ──
        # 1) 先把当前编辑目标的控件值收回权威配置：
        #    编辑全局 → 写 master 顶层映射段；编辑某设备 → 只把差异写进 devices[guid] 覆盖桶。
        # 2) cfg = master 深拷贝（含权威全局映射段 + 各设备覆盖桶）。
        #    映射段不再从控件重复收集（设备模式下控件呈现的是合并视图，直接写顶层会污染全局）。
        if not getattr(self, "_config_master", None):
            self._config_master = self._dc(self.cfg)
        self._flush_active_mapping()
        master = self._config_master
        cfg = self._dc(master)

        # combo 时间
        try:
            cfg["comboTime"] = int(self.combo_time_var.get())
        except (ValueError, AttributeError):
            cfg["comboTime"] = master.get("comboTime", 35)

        # 映射段（权威来自 master，flush 已写回）：全局顶层沿用；各设备差异在 devices 覆盖桶。
        cfg["comboMap"] = self._dc(master.get("comboMap", []))
        cfg["tapDance"] = self._dc(master.get("tapDance", dict(EMPTY_LAYERS)))
        cfg["devices"] = {k: v for k, v in (master.get("devices") or {}).items() if v}

        # 引擎选择

        # 设备订阅 + 别名
        if hasattr(self, '_device_rows'):
            aliases = dict(self.cfg.get("device_aliases", {}))
            per_dev = self._per_device_var.get()

            # ── 先遍历所有行，收集别名 + 开关状态（不区分 per_dev）──
            # 逐行的设备数据（供下方 per_dev 分支使用）
            _row_data = []  # [(guid,vid,pid,dtype,alias,enabled_on,dev,domain_var,dev_ids)]
            for row in self._device_rows:
                dev = row.get("device", {}) or {}
                raw_vid = row.get("vid") or dev.get("vendor_id") or dev.get("vid") or ""
                raw_pid = row.get("pid") or dev.get("product_id") or dev.get("pid") or ""
                vid = f"{int(raw_vid):04X}" if isinstance(raw_vid, int) else str(raw_vid).upper()
                pid = f"{int(raw_pid):04X}" if isinstance(raw_pid, int) else str(raw_pid).upper()
                dtype = row.get("type") or dev.get("device_type") or "keyboard"
                dtype = dtype.lower() if isinstance(dtype, str) else "keyboard"
                guid = row.get("guid") or dev.get("container_id") or dev.get("guid")
                alias = ""
                alias_var = row.get("alias") or row.get("alias_var")
                if alias_var is not None:
                    raw_alias = alias_var.get().strip()
                    default_name = dev.get("friendly_name") or dev.get("hardware_id") or ""
                    key = guid if guid else (f"{vid}:{pid}:{dtype}".upper() if (vid and pid) else None)
                    if key:
                        if raw_alias and raw_alias != default_name[:22]:
                            aliases[key] = raw_alias
                            alias = raw_alias
                        else:
                            aliases.pop(key, None)
                            alias = ""
                        if not guid and vid and pid:
                            aliases.pop(f"{vid}:{pid}", None)
                enabled_var = row.get("enabled") or row.get("switch")
                _on = bool(enabled_var.get()) if enabled_var is not None else True
                domain_var = row.get("domain")
                device_ids = row.get("device_ids", set())
                _online = row.get("online", True)
                _row_data.append((guid, vid, pid, dtype, alias, _on, dev, domain_var, device_ids, _online))

            def _make_entry(guid, vid, pid, dtype, alias, dev, domain_var, online=True, enabled=True):
                entry = {"vid": vid, "pid": pid, "type": dtype, "enabled": enabled,
                         "alias": alias, "online": online}
                if guid:
                    entry["guid"] = guid
                # 写入逐节点 hardware_id，供引擎 Tier-0 (GUID+HWID) 精确匹配（step-4 前置）
                _hw = dev.get("hardware_id") if isinstance(dev, dict) else None
                if _hw:
                    entry["hardware_id"] = _hw
                dev_id = dev.get("device_id") or dev.get("runtime_device_id")
                if dev_id:
                    try:
                        entry["runtime_device_id"] = int(dev_id)
                    except (ValueError, TypeError):
                        pass
                if domain_var is not None:
                    dom = domain_var.get().strip()
                    if dom:
                        _preset = {"独立":"0","全局":"1","域2":"2","域3":"3","域4":"4"}
                        if dom in _preset:
                            dom = _preset[dom]
                        try:
                            entry["domain_id"] = int(dom)
                        except ValueError:
                            entry["domain_id"] = 0
                return entry

            # ── 构建 device_registry（所有行） ──
            registry = []
            subscribed = []
            for (guid, vid, pid, dtype, alias, _on, dev, domain_var, device_ids, online) in _row_data:
                # 未启用独立设置 → 所有设备都经过全局映射（全部订阅）；
                # 启用独立设置 → 只订阅各设备开关打开的设备。
                _enabled = True if not per_dev else _on
                sub_devs = dev.get("_sub_devices") if isinstance(dev, dict) else None
                if sub_devs and isinstance(sub_devs, list) and len(sub_devs) > 1:
                    # 复合设备（同一 ContainerID 下多个 HID 子节点）：
                    # 为每个子节点写一条独立订阅，各自 hardware_id / type 不同。
                    for sd in sub_devs:
                        if sd.get("is_mouse"):
                            sd_type = "mouse"
                        elif sd.get("is_keyboard"):
                            sd_type = "keyboard"
                        else:
                            sd_type = "other"
                        reg = _make_entry(guid, vid, pid, sd_type, alias, sd, domain_var, online=online, enabled=_enabled)
                        registry.append(reg)
                        if _enabled:
                            subscribed.append(reg)
                else:
                    reg = _make_entry(guid, vid, pid, dtype, alias, dev, domain_var, online=online, enabled=_enabled)
                    registry.append(reg)
                    if _enabled:
                        subscribed.append(reg)
            cfg["device_registry"] = registry
            cfg["subscribed_devices"] = subscribed

            # 始终回写别名集合
            if aliases:
                cfg["device_aliases"] = aliases
            else:
                cfg.pop("device_aliases", None)

        # 引擎调试开关由系统托盘菜单负责（GUI 不放置），保存时保留 config 中已有值
        cfg["debug_enabled"] = master.get("debug_enabled", self.cfg.get("debug_enabled", False))

        # 保留驱动提示标记
        if "_driver_prompted" in master:
            cfg["_driver_prompted"] = master["_driver_prompted"]
        elif "_driver_prompted" in self.cfg:
            cfg["_driver_prompted"] = self.cfg["_driver_prompted"]

        # Tap Dance 全局时间（每层按键的 td 数据已随 layers 收集；时间参数保持全局，不做 per-device）
        try:
            ht_val = self._td_ht_var.get() if hasattr(self, '_td_ht_var') else master.get("tapDance", {}).get("holdTerm", 200)
            dt_val = self._td_dt_var.get() if hasattr(self, '_td_dt_var') else master.get("tapDance", {}).get("doubleTapTerm", 250)
            dh_val = self._td_dh_var.get() if hasattr(self, '_td_dh_var') else master.get("tapDance", {}).get("doubleHoldTerm", 200)
            # TD timing moved into layers
            cfg.setdefault("layers", {})
            cfg["tapDance"]["holdTerm"] = int(ht_val) if ht_val else 200
            cfg["tapDance"]["doubleTapTerm"] = int(dt_val) if dt_val else 250
            cfg["tapDance"]["doubleHoldTerm"] = int(dh_val) if dh_val else 200
        except Exception:
            cfg.setdefault("layers", {})
            cfg["tapDance"]["holdTerm"] = 200
            cfg["tapDance"]["doubleTapTerm"] = 250
            cfg["tapDance"]["doubleHoldTerm"] = 200
        # Leader 序列配置（Leader 系统始终开启）
        # timeouts/loopCapture 为全局级设置（非 per-device）；sequences 权威来自 master 全局
        # （flush 已把全局 sequences 写回 master，设备覆盖写在 devices 桶）。
        master_leader = master.get("leader", {}) or {}
        leader_cfg = dict(EMPTY_LEADER)
        leader_cfg.update({k: v for k, v in master_leader.items() if k != "sequences"})
        leader_cfg["loopCapture"] = self._leader_loop_capture_var.get() if hasattr(self, '_leader_loop_capture_var') else master_leader.get("loopCapture", False)
        try:
            leader_cfg["timeoutMs"] = int(self._leader_timeout_var.get()) if hasattr(self, '_leader_timeout_var') else master_leader.get("timeoutMs", EMPTY_LEADER["timeoutMs"])
        except (ValueError, AttributeError):
            leader_cfg["timeoutMs"] = master_leader.get("timeoutMs", EMPTY_LEADER["timeoutMs"])
        leader_cfg["sequences"] = self._dc(master_leader.get("sequences", []))
        cfg["leader"] = leader_cfg
        master["leader"] = self._dc(leader_cfg)

        # 收集期归一化：把 tap/hold/dt/dh 的裸键名包裹为 {X}（与 anykey-engine 规则一致）。
        # GUI 是规范化的唯一源头；引擎保留 wrap_single_key_output 作防御层。
        normalize_layers_key_outputs(cfg)

        # 记忆当前编辑目标（全局/某设备），重启时恢复
        cfg["lastEditTarget"] = self._edit_target
        # 记忆"设备独立设置"开关状态（perDevice：true=启用独立设置）
        cfg["perDevice"] = self._per_device_var.get()

        return cfg

    # ── 操作 ─────────────────────────────────
    def _save_all(self):
        cfg = self._collect_cfg()
        save_config(cfg)
        # self.cfg 保持合并视图，不替换为 master 深拷贝
        status = getattr(self, 'status_var', None)
        if status:
            status.set("✓ 配置已保存")

def _ensure_single_instance():
    """Windows 命名互斥体防止重复启动"""
    global _MUTEX
    try:
        import ctypes
        _MUTEX = ctypes.windll.kernel32.CreateMutexW(None, True, "AnyKey_SingleInstance")
        err = ctypes.windll.kernel32.GetLastError()
        if err == 183:  # ERROR_ALREADY_EXISTS
            # 找到之前的窗口并激活它（SW_RESTORE=9 恢复最小化/隐藏的窗口）
            import ctypes
            hwnd = ctypes.windll.user32.FindWindowW(None, "AnyKey - 任意键 配置管理器")
            if hwnd:
                ctypes.windll.user32.ShowWindow(hwnd, 9)  # SW_RESTORE
                ctypes.windll.user32.SetForegroundWindow(hwnd)
            sys.exit(0)
    except Exception:
        pass  # 出错时允许继续（比如非 Windows 环境）

# ──────────────────────────────────────────────
if __name__ == "__main__":
    _ensure_single_instance()
    try:
        app = AnyKeyApp()
        app.mainloop()
    except Exception as e:
        import traceback
        error_msg = f"错误: {type(e).__name__}: {e}\n\n"
        error_msg += traceback.format_exc()
        # 写入错误日志
        try:
            with open("error_log.txt", "w", encoding="utf-8") as f:
                f.write(error_msg)
        except:
            pass
        # 尝试显示错误对话框
        try:
            import tkinter.messagebox as mb
            mb.showerror("AnyKey 启动错误", error_msg)
        except:
            # 如果 messagebox 不可用，打印到 stderr
            import sys
            print(error_msg, file=sys.stderr)
        sys.exit(1)
