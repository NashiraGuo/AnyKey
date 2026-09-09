//! IPC 服务端 —— TCP 127.0.0.1:19527，JSON 行协议（与 lib/ipc.py 完全兼容）：
//!   请求：{"cmd":"pause"}\n
//!   响应：{"ok":true,...}\n 或 {"ok":false,"error":"..."}\n
//!   事件：{"event":"tray_shutdown"}\n / {"event":"state_changed","running":..,"paused":..}\n
//! 单连接模式（一次一个客户端），非阻塞 accept + 轮询以便随时退出。

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::app::Shared;

pub const IPC_HOST: &str = "127.0.0.1";
pub const IPC_PORT: u16 = 19527;
const POLL_MS: u64 = 50;

/// 启动 IPC 服务线程（listener 非阻塞轮询 + 每连接独立处理）
pub fn spawn(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind((IPC_HOST, IPC_PORT)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[tray] IPC bind 失败: {}", e);
                return;
            }
        };
        let _ = listener.set_nonblocking(true);

        while shared.running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    handle_client(&shared, stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(POLL_MS));
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
}

/// 处理单个客户端连接（阻塞读，500ms 超时轮询退出）
fn handle_client(shared: &Arc<Shared>, stream: TcpStream) {
    // 注册客户端供事件推送（读用原始 stream，写用 clone）
    // 命令响应与事件推送共用 shared.client 的锁，避免多线程写同一 socket 交错
    *shared.client.lock().unwrap() = stream.try_clone().ok();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        if !shared.running.load(Ordering::Relaxed) {
            break;
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // 连接关闭
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let resp = handle_request(shared, trimmed);
                let mut out = resp.to_string();
                out.push('\n');
                let mut guard = shared.client.lock().unwrap();
                if let Some(s) = guard.as_mut() {
                    let _ = s.write_all(out.as_bytes());
                }
                drop(guard);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => break,
        }
    }
    *shared.client.lock().unwrap() = None;
}

/// 解析请求并分发到 app 层
fn handle_request(shared: &Arc<Shared>, line: &str) -> serde_json::Value {
    let v: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return json!({"ok": false, "error": "invalid_json"}),
    };
    let cmd = v.get("cmd").and_then(|c| c.as_str()).unwrap_or("").to_string();

    match crate::app::handle_command(shared, &cmd) {
        Some(payload) => {
            let mut obj = serde_json::Map::new();
            obj.insert("ok".into(), json!(true));
            if let Some(extra) = payload.as_object() {
                for (k, val) in extra {
                    obj.insert(k.clone(), val.clone());
                }
            }
            serde_json::Value::Object(obj)
        }
        None => json!({"ok": false, "error": "unknown_command"}),
    }
}

/// 推送事件给已连接客户端（app 层调用）
pub fn send_event(shared: &Arc<Shared>, event: serde_json::Value) {
    let mut client = shared.client.lock().unwrap();
    if let Some(stream) = client.as_mut() {
        let mut out = event.to_string();
        out.push('\n');
        if stream.write_all(out.as_bytes()).is_err() {
            *client = None; // 写失败 → 断开
        }
    }
}
