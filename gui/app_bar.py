"""应用设置侧栏面板: 样式与设备栏一致。
Row1: 应用设置标题 + 右侧浏览按钮
Row2: 下拉菜单（点击自动刷新进程列表）
"""
import customtkinter as ctk
from tkinter import filedialog, messagebox
import os

try:
    import psutil
    HAS_PSUTIL = True
except ImportError:
    HAS_PSUTIL = False

# ── 常量 ──
THEME = None  # 由 main.py 注入
APP_NONE = "__NONE__"
FONT = ("Microsoft YaHei", 11)


def _set_theme(t):
    global THEME
    THEME = t


def _enum_foreground_processes() -> list[str]:
    """枚举当前有可见窗口的进程名（exe）。psutil 兜底全进程名。"""
    names: set[str] = set()
    if HAS_PSUTIL:
        try:
            import win32gui, win32process
            def _enum_cb(hwnd, _):
                if not win32gui.IsWindowVisible(hwnd):
                    return
                try:
                    _, pid = win32process.GetWindowThreadProcessId(hwnd)
                    p = psutil.Process(pid)
                    names.add(p.name().lower())
                except Exception:
                    pass
            win32gui.EnumWindows(_enum_cb, None)
        except ImportError:
            pass
    if not names and HAS_PSUTIL:
        for p in psutil.process_iter(['name']):
            try:
                names.add(p.info['name'].lower())
            except Exception:
                pass
    return sorted(names)


class AppPanel:
    """应用设置侧栏面板。两行：标题+浏览 / 下拉。"""

    def __init__(self, parent: ctk.CTkFrame, width: int = 192):
        self._parent = parent
        self._configured: set[str] = set()
        self._running_names: list[str] = []
        self._combo_display: list[str] = []
        self._combo_values: list[str] = []
        self._current = APP_NONE
        self._processes_loaded = False   # 进程列表是否已读过（首次必须读一次，见 _ensure_running_names）

        # 回调（main.py 注入）
        self.on_app_change = None

        # ── 容器：固定高度、无边框 ──
        self._frame = ctk.CTkFrame(parent, fg_color=THEME["card"],
                                   width=width, height=76)
        self._frame.pack(fill="x", padx=8, pady=(8, 4))
        self._frame.pack_propagate(False)

        # Row 1: 标题 + 浏览按钮
        row1 = ctk.CTkFrame(self._frame, fg_color="transparent")
        row1.pack(fill="x", padx=0, pady=(8, 2))
        ctk.CTkLabel(row1, text="应用设置", font=("Microsoft YaHei", 14, "bold"),
                     text_color=THEME["text_mid"]).pack(side="left")
        self._btn_browse = ctk.CTkButton(
            row1, text="浏览...", command=self._on_browse,
            fg_color="transparent", hover_color=THEME["border"],
            text_color=THEME["text_dark"], border_width=1,
            border_color=THEME["border"], corner_radius=5,
            width=58, height=22, font=FONT)
        self._btn_browse.pack(side="right")

        # Row 2: 下拉菜单（点击自动刷新）
        # 用 ctk.CTkComboBox 替代 ttk.Combobox，字号真实可控
        row2 = ctk.CTkFrame(self._frame, fg_color="transparent")
        row2.pack(fill="x", padx=0, pady=(2, 4))
        self._app_combo = ctk.CTkComboBox(
            row2, state="readonly", font=FONT,
            fg_color=THEME["card"], text_color=THEME["text_dark"],
            border_color=THEME["border"], border_width=1,
            button_color=THEME["border"], button_hover_color=THEME["text_mid"],
            dropdown_fg_color=THEME["card"],
            dropdown_text_color=THEME["text_dark"],
            dropdown_font=FONT,
            dropdown_hover_color=THEME["border"],
            height=28,
            command=self._on_select)
        self._app_combo.pack(fill="x")
        # 点箭头展开下拉 → 先刷新进程列表（未运行的应用已关、新开的应用要出现）。
        # **必须绑在 canvas 的箭头/右侧区域上**：CTkComboBox 的展开是
        # `canvas.tag_bind("dropdown_arrow", "<Button-1>", self._clicked)`，
        # 绑在 frame（self._app_combo.bind(...)）上的 <Button-1> 永远不会触发 ——
        # 点击落在子控件（entry / canvas item）上，Tk 不会把事件冒泡给父 frame。
        # add="+" 保留 CTk 自己的展开回调，只在我们之后/之前插一段刷新。
        self._bind_pre_refresh()

    def _bind_pre_refresh(self):
        for tag in ("dropdown_arrow", "right_parts"):
            try:
                self._app_combo._canvas.tag_bind(tag, "<Button-1>", self._pre_refresh, add="+")
            except Exception:
                pass

    # ── 公共接口 ──
    def set_configured_apps(self, names: list[str]):
        """更新「已配置」集合 —— 这就是 ● / ○ 圆点的唯一依据。

        只重建列表、**不每次重新枚举进程**：这个方法会被每次自动保存调用（编辑期随时在变），
        而枚举窗口进程要逐 hwnd 取 pid（win32gui + psutil），几十毫秒级，放在按键路径上会卡。
        但**首次必须读一次**（`_ensure_running_names`）—— 否则下拉里只有「全局」，
        运行中的应用一个都看不到，要等用户点开下拉才出现（2026-10-27 实测的启动回归）。
        """
        self._configured = set(names)
        self._ensure_running_names()
        self._rebuild()

    def _ensure_running_names(self):
        """进程列表惰性加载：只读一次，之后靠 refresh() / 点开下拉更新。"""
        if not self._processes_loaded:
            self._running_names = _enum_foreground_processes()
            self._processes_loaded = True

    def set_selected(self, app: str):
        self._current = app or APP_NONE

    def get_selected(self) -> str:
        return self._current if self._current != APP_NONE else ""

    def refresh(self):
        self._refresh_list()

    def _pre_refresh(self, _evt=None):
        """点击下拉时刷新进程列表（保持当前选中）。"""
        self._refresh_list()
        # 刷新后仍交给 ttk 展开下拉列表
        return None

    # ── 下拉更新 ──
    def _refresh_list(self):
        """重新枚举进程 + 重建列表（开销较大，只在"可能真的变了"的时机调用）。"""
        self._running_names = _enum_foreground_processes()
        self._processes_loaded = True
        self._rebuild()

    def _rebuild(self):
        """按当前 `_configured` / `_running_names` 重建下拉项与选中项（纯字符串，零 IO）。"""
        items = [("全局", APP_NONE)]
        for name in sorted(self._configured):
            items.append((f"● {name}", name))
        for name in self._running_names:
            if name not in self._configured:
                items.append((f"○ {name}", name))

        self._combo_display = [d for d, _ in items]
        self._combo_values = [v for _, v in items]
        self._app_combo.configure(values=self._combo_display)
        # 保持选中
        try:
            idx = self._combo_values.index(self._current)
            self._app_combo.set(self._combo_display[idx])
        except ValueError:
            self._app_combo.set(self._combo_display[0])

    def _on_select(self, display: str):
        """CTkComboBox command 回调：传入选中的 display 字符串。"""
        try:
            idx = self._combo_display.index(display)
        except ValueError:
            return
        self._current = self._combo_values[idx]
        if self.on_app_change:
            self.on_app_change(self.get_selected())

    def _on_browse(self):
        path = filedialog.askopenfilename(filetypes=[("可执行文件", "*.exe")])
        if not path:
            return
        name = os.path.basename(path).lower()
        if name not in self._combo_values:
            self._configured.add(name)
            self._refresh_list()
            try:
                idx = self._combo_values.index(name)
                self._app_combo.set(self._combo_display[idx])
                self._current = name
                if self.on_app_change:
                    self.on_app_change(name)
            except ValueError:
                pass
