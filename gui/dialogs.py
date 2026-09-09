"""
AnyKey - 对话框模块
配置编辑对话框
"""

import tkinter as tk
from tkinter import ttk, messagebox
import customtkinter as ctk
from gui.components import _THEME


def _place_at_top(win, parent):
    """定位：水平居中于父窗口、垂直贴父窗口顶部（不挡底部按键速查栏）。
    CTkToplevel 创建后 ~10ms 有一轮 withdraw→换标题栏色→deiconify 重绘，
    会冲掉直接设置的坐标；先 withdraw 隐藏、设好坐标，250ms 后再显示。"""
    win.withdraw()
    win.update_idletasks()
    x = parent.winfo_rootx() + max(0, (parent.winfo_width() - win.winfo_width()) // 2)
    y = parent.winfo_rooty() + 48
    scr_h = win.winfo_screenheight()
    if y + win.winfo_height() > scr_h - 40:
        y = max(0, scr_h - 40 - win.winfo_height())
    win.geometry(f"+{x}+{y}")
    win.after(250, win.deiconify)


class LargeInputDialog(ctk.CTkToplevel):
    """双击输入框弹出的大文本编辑窗（Combo / Tapdance / Leader 共用）。
    确认时回调 on_confirm(text)；取消或关窗不回调。
    调用方在 wait_window() 前把 self.textbox 设为按键速查插入目标。"""
    def __init__(self, parent, title, initial="", on_confirm=None):
        super().__init__(parent)
        self.title(title)
        self.resizable(True, True)
        self.transient(parent)
        # 不开 grab：开了之后键盘点击主窗口会被阻塞，用户切不回去
        self.attributes("-topmost", True)
        self._on_confirm = on_confirm

        self.textbox = ctk.CTkTextbox(self, font=("Consolas", 12),
                                      fg_color=_THEME["card_bg"],
                                      text_color=_THEME["text_dark"],
                                      border_color=_THEME["border"],
                                      border_width=1, corner_radius=4)
        self.textbox.pack(fill="both", expand=True, padx=12, pady=(12, 8))
        self.textbox.insert("1.0", initial)
        self.textbox.focus_set()

        btn_row = ctk.CTkFrame(self, fg_color="transparent")
        btn_row.pack(fill="x", padx=12, pady=(0, 12))
        ctk.CTkButton(btn_row, text="确认", command=self._confirm,
                      fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                      text_color="white", corner_radius=8,
                      width=100, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="right", padx=(8, 0))
        ctk.CTkButton(btn_row, text="取消", command=self.destroy,
                      fg_color=_THEME["card"], hover_color=_THEME["border"],
                      text_color=_THEME["text_dark"], corner_radius=8,
                      width=100, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="right")

        self.bind("<Escape>", lambda e: self.destroy())
        self.geometry("520x320")
        self.update_idletasks()
        _place_at_top(self, parent)

    def _confirm(self):
        text = self.textbox.get("1.0", "end-1c")
        if self._on_confirm:
            self._on_confirm(text)
        self.destroy()


class _RowDialog(ctk.CTkToplevel):
    def __init__(self, parent, title, fields, values):
        super().__init__(parent)
        self.title(title)
        self.resizable(False, False)
        self.grab_set()
        self.result = None

        card = ctk.CTkFrame(self, corner_radius=16)
        card.pack(padx=24, pady=24)

        self.entries = []
        for i, (label, val) in enumerate(zip(fields, values)):
            ctk.CTkLabel(card, text=label, font=("Microsoft YaHei", 12),
                         text_color=_THEME["text_mid"]).grid(
                             row=i, column=0, padx=(0, 8), pady=8, sticky="e")
            e = ctk.CTkEntry(card, font=("Consolas", 13), text_color="#3c3c3c",
                             fg_color=_THEME["card"], border_color=_THEME["border"],
                             border_width=1, corner_radius=6, width=260)
            e.insert(0, val)
            e.grid(row=i, column=1, padx=(0, 0), pady=8, sticky="w")
            self.entries.append(e)

        ctk.CTkLabel(card, text="AHK Send 格式示例：{Esc}  {Up}  {Backspace}",
                     font=("Microsoft YaHei", 10), text_color="#999999").grid(
                         row=len(fields), column=0, columnspan=2,
                         pady=(0, 12), sticky="w")

        btn_frame = ctk.CTkFrame(card, fg_color="transparent")
        btn_frame.grid(row=len(fields)+1, column=0, columnspan=2, pady=(0, 0))

        ctk.CTkButton(btn_frame, text="确认", command=self._ok,
                      fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                      text_color="white", corner_radius=8,
                      width=100, height=32, font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)
        ctk.CTkButton(btn_frame, text="取消", command=self.destroy,
                      fg_color=_THEME["card"], hover_color=_THEME["border"],
                      text_color=_THEME["text_dark"], corner_radius=8,
                      width=100, height=32, font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)

        self.entries[0].focus_set()
        self.bind("<Return>", lambda e: self._ok())
        self.bind("<Escape>", lambda e: self.destroy())

        self.update_idletasks()
        _place_at_top(self, parent)
        self.wait_window()

    def _ok(self):
        self.result = [e.get().strip() for e in self.entries]
        self.destroy()


class _KeyMapDialog(ctk.CTkToplevel):
    """编辑单条键映射：原键 → 映射输出"""
    def __init__(self, parent, title, key="", output=""):
        super().__init__(parent)
        self.title(title)
        self.resizable(False, False)
        self.grab_set()
        self.result = None

        card = ctk.CTkFrame(self, corner_radius=16)
        card.pack(padx=24, pady=24)

        ctk.CTkLabel(card, text="原键（AHK 按键名）：",
                     font=("Microsoft YaHei", 12),
                     text_color=_THEME["text_mid"]).grid(
                         row=0, column=0, padx=(0, 8), pady=8, sticky="e")
        self.key_entry = ctk.CTkEntry(card, font=("Consolas", 13),
                                      text_color=_THEME["text_dark"],
                                      fg_color=_THEME["card"],
                                      border_color=_THEME["border"],
                                      border_width=1, corner_radius=6,
                                      width=200)
        self.key_entry.insert(0, key)
        self.key_entry.grid(row=0, column=1, padx=(0, 0), pady=8, sticky="w")

        ctk.CTkLabel(card, text="映射输出（AHK Send 格式）：",
                     font=("Microsoft YaHei", 12),
                     text_color=_THEME["text_mid"]).grid(
                         row=1, column=0, padx=(0, 8), pady=8, sticky="e")
        self.out_entry = ctk.CTkEntry(card, font=("Consolas", 13),
                                      text_color=_THEME["text_dark"],
                                      fg_color=_THEME["card"],
                                      border_color=_THEME["border"],
                                      border_width=1, corner_radius=6,
                                      width=200)
        self.out_entry.insert(0, output)
        self.out_entry.grid(row=1, column=1, padx=(0, 0), pady=8, sticky="w")

        ctk.CTkLabel(card, text="示例：{Left}  {Down}  xfx  {Up}",
                     font=("Microsoft YaHei", 10),
                     text_color=_THEME["text_light"]).grid(
                         row=2, column=0, columnspan=2,
                         pady=(0, 12), sticky="w")

        btn_frame = ctk.CTkFrame(card, fg_color="transparent")
        btn_frame.grid(row=3, column=0, columnspan=2, pady=(0, 0))
        ctk.CTkButton(btn_frame, text="确认", command=self._ok,
                      fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                      text_color="white", corner_radius=8,
                      width=100, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)
        ctk.CTkButton(btn_frame, text="取消", command=self.destroy,
                      fg_color=_THEME["card"], hover_color=_THEME["border"],
                      text_color=_THEME["text_dark"], corner_radius=8,
                      width=100, height=32,
                      font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)

        self.key_entry.focus_set()
        self.bind("<Return>", lambda e: self._ok())
        self.bind("<Escape>", lambda e: self.destroy())

        self.update_idletasks()
        _place_at_top(self, parent)
        self.wait_window()

    def _ok(self):
        self.result = (self.key_entry.get().strip(),
                        self.out_entry.get().strip())
        self.destroy()



class _LayerDialog(ctk.CTkToplevel):
    """编辑层：层名称 + 键映射列表"""
    def __init__(self, parent, title, name=None, key_map=None):
        super().__init__(parent)
        self.title(title)
        self.resizable(False, False)
        self.grab_set()
        self.result = None
        self._key_map = dict(key_map) if key_map else {}

        card = ctk.CTkFrame(self, corner_radius=16)
        card.pack(padx=24, pady=24)

        # 层名称
        name_row = ctk.CTkFrame(card, fg_color="transparent")
        name_row.pack(fill="x", pady=(0, 10))
        ctk.CTkLabel(name_row, text="层名称：",
                     font=("Microsoft YaHei", 12),
                     text_color=_THEME["text_mid"]).pack(side="left")
        self.name_entry = ctk.CTkEntry(name_row, font=("Microsoft YaHei", 13),
                                        text_color=_THEME["text_dark"],
                                        fg_color=_THEME["card"],
                                        border_color=_THEME["border"],
                                        border_width=1, corner_radius=6,
                                        width=260)
        if name:
            self.name_entry.insert(0, name)
        self.name_entry.pack(side="left", padx=(4, 0))

        # 键映射列表
        ctk.CTkLabel(card, text="键映射列表：",
                     font=("Microsoft YaHei", 12, "bold"),
                     text_color=_THEME["text_mid"]).pack(anchor="w", pady=(0, 4))

        # Treeview for key mappings
        list_frame = ctk.CTkFrame(card, fg_color="transparent")
        list_frame.pack(fill="both", expand=True, pady=(0, 8))

        cols = ("key", "output")
        self.tree = ttk.Treeview(list_frame, columns=cols, show="headings",
                                  selectmode="browse", height=6)
        style = ttk.Style()
        style.theme_use("clam")
        style.configure("Treeview",
                        background=_THEME["card"],
                        foreground=_THEME["text_dark"],
                        fieldbackground=_THEME["card"],
                        rowheight=36, font=("Segoe UI", 15))
        style.configure("Treeview.Heading",
                        background="", foreground=_THEME["text_dark"],
                        font=("Segoe UI", 13, "bold"))
        self.tree.heading("key", text="原键")
        self.tree.column("key", width=160, anchor="center")
        self.tree.heading("output", text="映射输出")
        self.tree.column("output", width=260, anchor="center")
        self.tree.pack(side="left", fill="both", expand=True)

        # 按钮行
        btn_row = ctk.CTkFrame(card, fg_color="transparent")
        btn_row.pack(pady=(0, 0))

        def add_mapping():
            dlg = _KeyMapDialog(self, "添加键映射")
            if dlg.result:
                k, o = dlg.result
                if not k:
                    messagebox.showerror("错误", "原键不能为空", parent=self)
                    return
                self._key_map[k] = o
                self._refresh_tree()

        def edit_mapping():
            sel = self.tree.selection()
            if not sel:
                messagebox.showinfo("提示", "请先选择一条映射", parent=self)
                return
            key = self.tree.item(sel[0], "values")[0]
            dlg = _KeyMapDialog(self, "编辑键映射", key, self._key_map.get(key, ""))
            if dlg.result:
                k, o = dlg.result
                if not k:
                    messagebox.showerror("错误", "原键不能为空", parent=self)
                    return
                del self._key_map[key]
                self._key_map[k] = o
                self._refresh_tree()

        def del_mapping():
            sel = self.tree.selection()
            if not sel:
                return
            key = self.tree.item(sel[0], "values")[0]
            if messagebox.askyesno("确认", f"删除映射「{key}」？", parent=self):
                del self._key_map[key]
                self._refresh_tree()

        ctk.CTkButton(btn_row, text="添加", command=add_mapping,
                       fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                       text_color="white", corner_radius=8,
                       width=88, height=32,
                       font=("Microsoft YaHei", 13, "bold")).pack(side="left")
        ctk.CTkButton(btn_row, text="编辑", command=edit_mapping,
                       fg_color=_THEME["blue"], hover_color=_THEME["blue_h"],
                       text_color="white", corner_radius=8,
                       width=88, height=32,
                       font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)
        ctk.CTkButton(btn_row, text="删除", command=del_mapping,
                       fg_color=_THEME["red"], hover_color=_THEME["red_h"],
                       text_color="white", corner_radius=8,
                       width=88, height=32,
                       font=("Microsoft YaHei", 13, "bold")).pack(side="left")

        # 确认 / 取消
        btn_frame = ctk.CTkFrame(card, fg_color="transparent")
        btn_frame.pack(pady=(12, 0))
        ctk.CTkButton(btn_frame, text="确认",
                       command=self._ok,
                       fg_color=_THEME["green"], hover_color=_THEME["green_h"],
                       text_color="white", corner_radius=8,
                       width=100, height=32,
                       font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)
        ctk.CTkButton(btn_frame, text="取消",
                       command=self.destroy,
                       fg_color=_THEME["card"], hover_color=_THEME["border"],
                       text_color=_THEME["text_dark"], corner_radius=8,
                       width=100, height=32,
                       font=("Microsoft YaHei", 13, "bold")).pack(side="left", padx=8)

        self._refresh_tree()
        self.name_entry.focus_set()
        self.bind("<Escape>", lambda e: self.destroy())

        self.update_idletasks()
        _place_at_top(self, parent)
        self.wait_window()

    def _refresh_tree(self):
        self.tree.delete(*self.tree.get_children())
        for k, o in self._key_map.items():
            self.tree.insert("", "end", values=(k, o))

    def _ok(self):
        name = self.name_entry.get().strip()
        if not name:
            messagebox.showerror("错误", "层名称不能为空", parent=self)
            return
        self.result = (name, dict(self._key_map))
        self.destroy()

