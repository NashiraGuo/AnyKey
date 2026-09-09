"""
AnyKey - GUI 组件模块
自定义 Widget 和颜色主题
"""

import tkinter as tk
import customtkinter as ctk

_THEME = dict(
    green="#5d8a6d",   green_h="#4a7a5a",
    blue="#6272a0",    blue_h="#505f8a",
    red="#c76a6a",     red_h="#b55555",
    orange="#c89450",  orange_h="#b07d3a",
    purple="#8b6b9e",  purple_h="#74558a",
    card="#d8d8d8",
    border="#bebebe",
    text_dark="#2a2a2a", text_mid="#5a5a5a", text_light="#888888",
    text_disabled="#b0b0b0",
    status_ok="#3b6d11", status_miss="#a32d2d",
    card_bg="#f5f5f5", card_hover="#e8e8e8", card_active="#e6f1fb",
    green_bg="#eaf3de",
    row_bg="#e0e0e0",
    # per-device 映射覆盖配色
    indigo="#6C8CFF", indigo_h="#5877e6",   # 全局设置伪条目
    amber="#EF9F27",                          # 继承全局（badge）
    override="#3FB68B",                       # 设备覆盖（badge）
    inherit="#1E5FBF",                        # 继承自全局的条目边框（深蓝）
)

class _CapsuleScrollbar(ctk.CTkFrame):
    def __init__(self, master, orient="vertical", command=None,
                 bg_color="#c8c8c8", slider_color="#a0a0a0",
                 track_width=3, slider_thickness=8, auto_hide=False, **kwargs):
        super().__init__(master, fg_color="transparent", **kwargs)
        self.orient = orient
        self.command = command
        self.bg_color = bg_color
        self.slider_color = slider_color
        self.track_width = track_width
        self.slider_thickness = slider_thickness
        self._auto_hide = auto_hide
        self._fraction = 0.0   # 0.0 ~ 1.0 滑块顶部位置
        self._span = 0.2       # 滑块占轨道比例
        self._dragging = False

        w = 10 if orient == "vertical" else 0
        h = 0 if orient == "vertical" else 10
        self.canvas = ctk.CTkCanvas(
            self, bg=bg_color, highlightthickness=0, bd=0,
            width=w, height=h)
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Button-1>", self._click)
        self.canvas.bind("<B1-Motion>", self._drag)
        self.canvas.bind("<Button-4>", lambda e: self._scroll(-1))
        self.canvas.bind("<Button-5>", lambda e: self._scroll(1))
        self.canvas.bind("<MouseWheel>", self._wheel)

    def set(self, *args):
        first, last = float(args[0]), float(args[1])
        self._fraction = first
        self._span = max(0.05, last - first)
        self._redraw()
        if self._auto_hide:
            if abs(first - 0.0) < 0.001 and abs(last - 1.0) < 0.001:
                if self.winfo_ismapped():
                    self.grid_remove()
            else:
                if not self.winfo_ismapped():
                    self.grid()

    def _on_scroll(self):
        self._redraw()

    def _rrect(self, c, x1, y1, x2, y2, r, **kw):
        """用 arc + rect 画圆角矩形（不依赖 outline 参数，直接画边框）"""
        fill = kw.pop("fill", "")
        # 直接画圆角矩形，不画边框（滑块为纯色胶囊形）
        # 四个圆角
        c.create_arc(x1, y1, x1+2*r, y1+2*r, start=90, extent=90, fill=fill, outline="")
        c.create_arc(x2-2*r, y1, x2, y1+2*r, start=0, extent=90, fill=fill, outline="")
        c.create_arc(x1, y2-2*r, x1+2*r, y2, start=180, extent=90, fill=fill, outline="")
        c.create_arc(x2-2*r, y2-2*r, x2, y2, start=270, extent=90, fill=fill, outline="")
        # 中间矩形
        c.create_rectangle(x1+r, y1, x2-r, y2, fill=fill, outline="")
        c.create_rectangle(x1, y1+r, x2, y2-r, fill=fill, outline="")

    def _redraw(self):
        c = self.canvas
        c.delete("all")
        W = c.winfo_width() or 10
        H = c.winfo_height() or 200
        tw = self.track_width
        st = self.slider_thickness
        R = st // 2

        if self.orient == "vertical":
            cx = W // 2
            # 轨道槽
            c.create_line(cx, R, cx, H - R,
                         fill="#b0b0b0", width=tw, capstyle="round")
            # 滑块
            track_h = H - 2 * R
            # 确保 span 在合理范围内
            span = max(0.05, min(1.0, self._span))
            sh = max(st, span * track_h)
            sh = min(sh, track_h)  # 滑块高度不能超过轨道高度
            
            if span >= 1.0:
                sy = R
            else:
                # 计算滑块位置（归一化）
                frac = max(0.0, min(1.0 - span, self._fraction))
                norm = frac / max(0.001, 1.0 - span)
                sy = R + norm * (track_h - sh)
            
            # 确保滑块在轨道范围内
            sy = max(R, min(H - R - sh, sy))
            
            self._rrect(c, cx - R, sy, cx + R, sy + sh, R,
                       fill=self.slider_color, outline="")
        else:
            cy = H // 2
            c.create_line(R, cy, W - R, cy,
                         fill="#b0b0b0", width=tw, capstyle="round")
            track_w = W - 2 * R
            span = max(0.05, min(1.0, self._span))
            sw = max(st, span * track_w)
            sw = min(sw, track_w)
            
            if span >= 1.0:
                sx = R
            else:
                frac = max(0.0, min(1.0 - span, self._fraction))
                norm = frac / max(0.001, 1.0 - span)
                sx = R + norm * (track_w - sw)
            
            sx = max(R, min(W - R - sw, sx))
            
            self._rrect(c, sx, cy - R, sx + sw, cy + R, R,
                       fill=self.slider_color, outline="")

    def _click(self, e):
        c = self.canvas
        W, H = c.winfo_width(), c.winfo_height()
        R = self.slider_thickness // 2

        if self.orient == "vertical":
            track_h = H - 2 * R
            span = max(0.05, min(1.0, self._span))
            sh = max(self.slider_thickness, span * track_h)
            sh = min(sh, track_h)
            if span >= 1.0:
                slider_start = R
            else:
                frac_pos = max(0.0, min(1.0 - span, self._fraction))
                norm_p = frac_pos / max(0.001, 1.0 - span)
                slider_start = R + norm_p * (track_h - sh)
            slider_end = slider_start + sh
            if slider_start <= e.y <= slider_end:
                self._drag_offset = e.y - slider_start
                self._dragging = True
            elif e.y < slider_start:
                self._scroll(1)
            else:
                self._scroll(-1)
        else:
            track_w = W - 2 * R
            span = max(0.05, min(1.0, self._span))
            sw = max(self.slider_thickness, span * track_w)
            sw = min(sw, track_w)
            if span >= 1.0:
                slider_start = R
            else:
                frac_pos = max(0.0, min(1.0 - span, self._fraction))
                norm_p = frac_pos / max(0.001, 1.0 - span)
                slider_start = R + norm_p * (track_w - sw)
            slider_end = slider_start + sw
            if slider_start <= e.x <= slider_end:
                self._drag_offset = e.x - slider_start
                self._dragging = True
            elif e.x < slider_start:
                self._scroll(1)
            else:
                self._scroll(-1)

    def _drag(self, e):
        if not self._dragging:
            return
        c = self.canvas
        W, H = c.winfo_width(), c.winfo_height()
        R = self.slider_thickness // 2
        if self.orient == "vertical":
            pos = (e.y - R - self._drag_offset) / max(1, H - 2 * R)
        else:
            pos = (e.x - R - self._drag_offset) / max(1, W - 2 * R)
        frac = max(0, min(1, pos / max(0.001, 1 - self._span)))
        if self.command:
            self.command("moveto", str(frac))

    def _scroll(self, delta):
        step = 0.05 * delta
        frac = (self._fraction + step) / max(0.001, 1 - self._span)
        frac = max(0, min(1, frac))
        if self.command:
            self.command("moveto", str(frac))

    def _wheel(self, e):
        self._scroll(-1 if e.delta > 0 else 1)


