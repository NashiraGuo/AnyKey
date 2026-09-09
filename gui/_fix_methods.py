"""Replace delete/import/export methods in main.py"""
path = 'main.py'
with open(path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

new_block = """\
    def _on_delete_page_clicked(self):
        label = self._current_app or "全局"
        if not messagebox.askyesno(
                "删除当前页设置",
                f"确定清空 [{label}] 当前页面的设置吗？\\n"
                f"（如需清空全部设置，请在 Combo / TapDance / Leader 三页分别删除）\\n"
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
"""

# Find the start and end of the block
start = None
for i, line in enumerate(lines):
    if line.startswith('    def _on_delete_page_clicked(self):'):
        start = i
        break

end = None
for i in range(start + 1, len(lines)):
    line = lines[i].strip()
    # Find next top-level method: def _badge_sets or similar
    if (i > start + 2 and lines[i].startswith('    def _') and 
        not lines[i].startswith('    def _delete_app_page') and
        not lines[i].startswith('    def _apply_badge_to')):
        end = i
        break
    # Fallback: find the next method after _reload_current_page
    if i > start and 'def _badge_sets' in lines[i]:
        end = i
        break

if end is None:
    end = start
    for i in range(start + 1, len(lines)):
        if lines[i].startswith('    def _') and '_on_delete' not in lines[i] and '_export_app' not in lines[i] and '_import_app' not in lines[i] and '_delete_app_page' not in lines[i] and '_clear_page' not in lines[i] and '_collect_page' not in lines[i] and '_apply_page_data' not in lines[i] and '_reload_current' not in lines[i]:
            if '_reload_view' not in lines[i] and '_apply_page_data_to_container' not in lines[i]:
                end = i
                break

print(f"Replacing lines {start+1}-{end}")

result = lines[:start] + [new_block] + lines[end:]
with open(path, 'w', encoding='utf-8') as f:
    f.writelines(result)
print("done")
