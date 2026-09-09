"""兼容性测试：用 GUI 真实客户端（lib/ipc.py IpcClient）驱动 Rust tray"""
import os
import sys
import time

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))
from lib.ipc import IpcClient, CMD_PAUSE, CMD_RESUME, CMD_RELOAD, CMD_STATUS

events = []


def on_event(name, payload):
    events.append((name, payload))


def main():
    print("=== 用真实 GUI 客户端（lib/ipc.py）测试 Rust tray ===")
    client = IpcClient(on_event=on_event)
    assert client.wait_for_server(timeout=5), "连接 tray 失败"
    print("[PASS] 连接成功")

    checks = []

    r = client.send(CMD_STATUS)
    checks.append(r.get("ok") and "running" in r and "paused" in r and "debug" in r)
    print(f"  status -> {r}")

    r = client.send(CMD_PAUSE)
    checks.append(r.get("ok") and r.get("paused") is True)
    print(f"  pause  -> {r}")

    r = client.send(CMD_STATUS)
    checks.append(r.get("paused") is True)
    print(f"  status -> {r}")

    r = client.send(CMD_RESUME)
    checks.append(r.get("ok") and r.get("paused") is False)
    print(f"  resume -> {r}")

    r = client.send(CMD_RELOAD)
    checks.append(r.get("ok") and r.get("running") is True)
    print(f"  reload -> {r}")

    # 等待事件推送（subscribe 后应有 state_changed）
    time.sleep(0.5)
    state_events = [e for e in events if e[0] == "state_changed"]
    checks.append(len(state_events) >= 3)
    print(f"  state_changed 事件数: {len(state_events)}（pause/resume/reload 各一次）")

    passed = sum(checks)
    print(f"\n结果: {passed}/{len(checks)} PASS")
    client.disconnect()
    sys.exit(0 if passed == len(checks) else 1)


if __name__ == "__main__":
    main()
