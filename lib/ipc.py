"""
AnyKey IPC — TCP localhost + JSON 行协议
单连接，一个 reader 线程统管所有读写，send() 不碰 socket
"""

import json
import socket
import threading
import time
import logging

logger = logging.getLogger(__name__)

# ── 常量 ──────────────────────────────────────
IPC_HOST = "127.0.0.1"
IPC_PORT = 19527
CONNECT_TIMEOUT = 3.0
REQUEST_TIMEOUT = 5.0
BUFFER_SIZE = 8192

# ── 命令 ──────────────────────────────────────
CMD_PAUSE = "pause"
CMD_RESUME = "resume"
CMD_RELOAD = "reload"
CMD_STATUS = "status"
CMD_QUIT = "quit"
CMD_PING = "ping"
CMD_SUBSCRIBE = "subscribe"

ALL_COMMANDS = {CMD_PAUSE, CMD_RESUME, CMD_RELOAD, CMD_STATUS, CMD_QUIT, CMD_PING, CMD_SUBSCRIBE}


def _parse_line(line: bytes):
    """解析一行 JSON"""
    text = line.decode("utf-8", errors="replace").strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


# ══════════════════════════════════════════════
#  IpcServer — 托盘侧
# ══════════════════════════════════════════════

class IpcServer:
    """IPC 服务端：监听 localhost，一次一个客户端"""

    def __init__(self, handler):
        self._handler = handler
        self._socket = None
        self._client = None
        self._thread = None
        self._running = False

    def start(self):
        if self._running:
            return
        self._running = True
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        logger.info("IPC server started on %s:%d", IPC_HOST, IPC_PORT)

    def stop(self):
        self._running = False
        for s in [self._socket, self._client]:
            if s:
                try:
                    s.close()
                except Exception:
                    pass
        self._socket = None
        self._client = None
        logger.info("IPC server stopped")

    def send_event(self, event_name: str, **kwargs):
        """主动推送事件给已连接的客户端"""
        client = self._client
        if client is None:
            return
        payload = json.dumps(dict(event=event_name, **kwargs)) + "\n"
        try:
            client.sendall(payload.encode("utf-8"))
        except Exception:
            pass

    def _run(self):
        while self._running:
            try:
                self._socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                self._socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                self._socket.settimeout(1.0)
                self._socket.bind((IPC_HOST, IPC_PORT))
                self._socket.listen(1)

                while self._running:
                    try:
                        client, addr = self._socket.accept()
                        logger.info("IPC client connected from %s", addr)
                        self._client = client
                        self._handle_client(client)
                    except socket.timeout:
                        continue
                    except OSError:
                        break
            except OSError:
                if self._running:
                    time.sleep(1)
            finally:
                if self._socket:
                    try:
                        self._socket.close()
                    except Exception:
                        pass
                self._socket = None

    def _handle_client(self, client):
        buf = b""
        try:
            client.settimeout(0.5)
            while self._running:
                try:
                    data = client.recv(BUFFER_SIZE)
                except socket.timeout:
                    continue
                except OSError:
                    break
                if not data:
                    break
                buf += data
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    msg = _parse_line(line)
                    if msg is None:
                        continue
                    cmd = msg.get("cmd", "")
                    if cmd not in ALL_COMMANDS:
                        client.sendall(b'{"ok":false,"error":"unknown_command"}\n')
                        continue
                    try:
                        result = self._handler(cmd)
                        if not isinstance(result, dict):
                            result = {}
                        client.sendall((json.dumps({"ok": True, **result}) + "\n").encode())
                    except Exception as e:
                        client.sendall((json.dumps({"ok": False, "error": str(e)}) + "\n").encode())
        except Exception:
            pass
        finally:
            self._client = None
            try:
                client.close()
            except Exception:
                pass
            logger.info("IPC client disconnected")


# ══════════════════════════════════════════════
#  IpcClient — GUI 侧
# ══════════════════════════════════════════════

class IpcClient:
    """IPC 客户端：一个 reader 线程管所有读写，send() 用 condition 等响应"""

    def __init__(self, on_event=None):
        self._on_event = on_event
        self._socket = None
        self._lock = threading.Lock()
        self._cond = threading.Condition()
        self._response = None
        self._reader_thread = None
        self._running = False

    def is_connected(self):
        return self._socket is not None

    def connect(self):
        if self._socket:
            return True
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(CONNECT_TIMEOUT)
            sock.connect((IPC_HOST, IPC_PORT))
            sock.settimeout(None)  # 连接成功后切阻塞模式，reader 线程不会超时退出
            self._socket = sock
            self._running = True
            self._reader_thread = threading.Thread(target=self._reader_loop, daemon=True)
            self._reader_thread.start()
            # 订阅事件推送
            self.send(CMD_SUBSCRIBE)
            return True
        except (OSError, TimeoutError):
            return False

    def disconnect(self):
        self._running = False
        sock = self._socket
        self._socket = None
        if sock:
            try:
                sock.close()
            except Exception:
                pass

    def send(self, cmd: str, timeout: float = REQUEST_TIMEOUT) -> dict:
        """发送命令，等待 reader 线程收到响应后返回"""
        if cmd not in ALL_COMMANDS:
            return {"ok": False, "error": "unknown_command"}
        if not self._socket:
            return {"ok": False, "error": "not_connected"}

        with self._cond:
            sock = self._socket
            if sock is None:
                return {"ok": False, "error": "not_connected"}

            self._response = None
            payload = json.dumps({"cmd": cmd}) + "\n"
            try:
                sock.sendall(payload.encode("utf-8"))
            except OSError:
                self.disconnect()
                return {"ok": False, "error": "io_error"}

            if not self._cond.wait(timeout):
                return {"ok": False, "error": "timeout"}

            return self._response or {"ok": False, "error": "empty_response"}

    def _reader_loop(self):
        """后台线程：持续读 socket，区分响应和事件"""
        buf = b""
        while self._running:
            sock = self._socket
            if sock is None:
                break
            try:
                data = sock.recv(BUFFER_SIZE)
            except socket.timeout:
                continue  # 超时不是断开，继续等
            except OSError:
                break  # 真正的 IO 错误才退出
            if not data:
                break  # 连接被对方关闭

            buf += data
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                msg = _parse_line(line)
                if msg is None:
                    continue

                if "event" in msg:
                    # 事件 → 回调
                    if self._on_event:
                        try:
                            self._on_event(msg["event"], msg)
                        except Exception:
                            pass
                elif "ok" in msg:
                    # 响应 → 通知等待的 send()
                    with self._cond:
                        self._response = msg
                        self._cond.notify_all()

        self._running = False
        self._socket = None

    def wait_for_server(self, timeout: float = 5.0, interval: float = 0.3):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.connect():
                return True
            time.sleep(interval)
        return False
