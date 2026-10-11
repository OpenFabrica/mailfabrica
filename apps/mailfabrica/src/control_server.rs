//! 本机控制服务器（JSON 行）。每个连接可以连续发请求：一行 `{"id":…, "method":…, "params":…}`，
//! 回一行 `{"id":…, "ok":…, …}`。方法列表见 `mailfabrica_ui_egui::control`。

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use mailfabrica_ui_egui::ControlRequest;
use serde_json::{Value as Json, json};

/// 单行请求的上限（附件 base64 可能很大，但不能无限）。
const MAX_LINE: usize = 64 * 1024 * 1024;

/// 在 127.0.0.1:`port` 上监听，返回界面线程取请求的接收端。
pub fn start(port: u16, ctx: egui::Context) -> std::io::Result<Receiver<ControlRequest>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (tx, rx) = channel();
    std::thread::Builder::new().name("control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            let _ = std::thread::Builder::new().name("control-conn".into()).spawn(move || serve(stream, tx, ctx));
        }
    })?;
    eprintln!("MailFabrica control channel listening on 127.0.0.1:{port}");
    Ok(rx)
}

/// 处理一个连接：逐行读请求，转给界面线程，等回复写回去。
fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = if line.len() > MAX_LINE {
            json!({"ok": false, "error": "request too large"})
        } else {
            match serde_json::from_str::<Json>(&line) {
                Err(e) => json!({"ok": false, "error": format!("invalid JSON: {e}")}),
                Ok(msg) => {
                    let id = msg.get("id").cloned().unwrap_or(Json::Null);
                    let method = msg.get("method").and_then(Json::as_str).unwrap_or("").to_string();
                    let params = msg.get("params").cloned().unwrap_or(json!({}));
                    let (req, rx) = ControlRequest::new(method, params);
                    if tx.send(req).is_err() {
                        break;
                    }
                    // egui 只在有事件时才跑一帧：唤醒界面线程
                    ctx.request_repaint();
                    let mut r = rx
                        .recv_timeout(Duration::from_secs(60))
                        .unwrap_or_else(|_| json!({"ok": false, "error": "timed out waiting for the UI (is the window minimised?)"}));
                    if let Some(o) = r.as_object_mut() {
                        o.insert("id".into(), id);
                    }
                    r
                }
            }
        };
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}
