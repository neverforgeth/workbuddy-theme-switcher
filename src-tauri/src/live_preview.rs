//! Loopback-only CDP transport, version-bound captures, and local screenshot consistency.
use super::*;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::io::Cursor;
use std::io::{self, Read, Write};

struct DeadlineIo<S> {
    // Production always supplies a nonblocking TcpStream. The deadline also guards the
    // I/O loops inside tungstenite, which do not necessarily yield a whole message.
    inner: S,
    deadline: Instant,
}
impl<S> DeadlineIo<S> {
    fn check_deadline(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            Err(io::ErrorKind::TimedOut.into())
        } else {
            Ok(())
        }
    }
}
impl<S: Read> Read for DeadlineIo<S> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.check_deadline()?;
        let result = self.inner.read(buffer);
        self.check_deadline()?;
        result
    }
}
impl<S: Write> Write for DeadlineIo<S> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.check_deadline()?;
        let result = self.inner.write(buffer);
        self.check_deadline()?;
        result
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check_deadline()?;
        let result = self.inner.flush();
        self.check_deadline()?;
        result
    }
}

pub(crate) fn css_hash(css: &str) -> String {
    format!("{:x}", Sha256::digest(css.as_bytes()))
}
fn err() -> AppError {
    AppError::new(
        "LIVE_PREVIEW_FAILED",
        "实机画面未更新，请检查连接后刷新；旧画面不代表当前参数。",
    )
}
pub(crate) struct Cdp {
    socket: WebSocket<DeadlineIo<TcpStream>>,
    next: u64,
    pub target_id: String,
}
impl Cdp {
    pub fn connect(port: u16) -> AppResult<Self> {
        let target = discover_renderer(port)?;
        let url = url::Url::parse(&target.websocket_debugger_url).map_err(|_| err())?;
        if url.scheme() != "ws"
            || !matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            || url.port() != Some(port)
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(err());
        }
        let address = format!("127.0.0.1:{port}").parse().map_err(|_| err())?;
        let stream =
            TcpStream::connect_timeout(&address, Duration::from_millis(500)).map_err(|_| err())?;
        let mut socket = handshake(stream, url.as_str(), Duration::from_secs(3))?;
        configure_socket(&mut socket)?;
        Ok(Self {
            socket,
            next: 0,
            target_id: target.target_id,
        })
    }
    pub fn call(&mut self, method: &str, params: Value) -> AppResult<Value> {
        let timeout = if method == "Page.captureScreenshot" {
            Duration::from_secs(8)
        } else {
            Duration::from_secs(3)
        };
        self.call_with_timeout(method, params, timeout)
    }
    fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> AppResult<Value> {
        self.next += 1;
        let id = self.next;
        let start = Instant::now();
        self.socket.get_mut().deadline = start + timeout;
        // Enforce the budget inside every nonblocking I/O, not only between messages:
        // tungstenite may consume continuously ready fragments without returning here.
        match self.socket.send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        )) {
            Ok(()) => {}
            Err(e) if would_block(&e) => {} // buffered once; never resend a command
            Err(_) => return Err(err()),
        }
        loop {
            if start.elapsed() >= timeout {
                return Err(err());
            }
            match self.socket.flush() {
                Ok(()) => {}
                Err(e) if would_block(&e) => {}
                Err(_) => return Err(err()),
            }
            let message = match self.socket.read() {
                Ok(message) => message,
                Err(e) if would_block(&e) => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => return Err(err()),
            };
            match message {
                Message::Text(s) => {
                    if s.len() > 24 * 1024 * 1024 {
                        return Err(err());
                    }
                    let v: Value = serde_json::from_str(&s).map_err(|_| err())?;
                    if v["id"].as_u64() != Some(id) {
                        continue;
                    }
                    if v.get("error").is_some() {
                        return Err(err());
                    }
                    self.socket.get_ref().check_deadline().map_err(|_| err())?;
                    return Ok(v["result"].clone());
                }
                Message::Ping(_) => {} // tungstenite queues the pong; next iteration flushes it
                Message::Close(_) => return Err(err()),
                _ => {}
            }
        }
    }
    pub fn evaluate(&mut self, expression: &str) -> AppResult<Value> {
        let v=self.call("Runtime.evaluate",json!({"expression":expression,"returnByValue":true,"awaitPromise":true,"timeout":2500}))?;
        if v.get("exceptionDetails").is_some() {
            return Err(err());
        }
        v.pointer("/result/value").cloned().ok_or_else(err)
    }
    pub fn verify(&mut self, expected_id: &str, expected_css: &str) -> AppResult<()> {
        let value=self.evaluate("(() => ({count:document.querySelectorAll('#codedrobe-theme-style-workbuddy').length,id:document.documentElement.dataset.codedrobeTheme,css:document.getElementById('codedrobe-theme-style-workbuddy')?.textContent}))()")?;
        if value["count"] != 1
            || value["id"].as_str() != Some(expected_id)
            || value["css"].as_str() != Some(expected_css)
        {
            return Err(AppError::new(
                "LIVE_THEME_CHANGED",
                "实机主题已变化，停止同步并准备恢复。",
            ));
        }
        Ok(())
    }
    pub fn theme_fingerprint(&mut self) -> AppResult<(u64, Option<String>, Option<String>)> {
        let value=self.evaluate("(() => ({count:document.querySelectorAll('#codedrobe-theme-style-workbuddy').length,id:document.documentElement.dataset.codedrobeTheme,css:document.getElementById('codedrobe-theme-style-workbuddy')?.textContent}))()")?;
        Ok((
            value["count"].as_u64().ok_or_else(err)?,
            value["id"].as_str().map(str::to_string),
            value["css"].as_str().map(css_hash),
        ))
    }
    pub fn update(&mut self, id: &str, old_css: &str, css: &str, sequence: u64) -> AppResult<()> {
        self.verify(id, old_css)?;
        let css_json = serde_json::to_string(css).map_err(|_| err())?;
        let id_json = serde_json::to_string(id).map_err(|_| err())?;
        let expression=format!("(() => {{const s=document.getElementById('codedrobe-theme-style-workbuddy');const runtime=window.__CODEDROBE__?.hosts?.workbuddy;if(!runtime?.updateCss?.({id_json},{css_json}))throw new Error('Live update unsupported');s.dataset.studioSequence='{sequence}';return true;}})()");
        self.evaluate(&expression)?;
        self.verify(id, css)
    }
}
fn handshake(
    stream: TcpStream,
    url: &str,
    timeout: Duration,
) -> AppResult<WebSocket<DeadlineIo<TcpStream>>> {
    stream.set_nonblocking(true).map_err(|_| err())?;
    let deadline = Instant::now() + timeout;
    let mut attempt = client(
        url,
        DeadlineIo {
            inner: stream,
            deadline,
        },
    );
    loop {
        if Instant::now() >= deadline {
            return Err(err());
        }
        match attempt {
            Ok((socket, _)) => return Ok(socket),
            Err(tungstenite::HandshakeError::Interrupted(pending)) => {
                std::thread::sleep(Duration::from_millis(5));
                attempt = pending.handshake();
            }
            Err(tungstenite::HandshakeError::Failure(_)) => return Err(err()),
        }
    }
}
fn would_block(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock)
}
fn configure_socket(socket: &mut WebSocket<DeadlineIo<TcpStream>>) -> AppResult<()> {
    socket.set_config(|config| {
        config.max_message_size = Some(24 * 1024 * 1024);
        config.max_frame_size = Some(24 * 1024 * 1024);
        config.max_write_buffer_size = 4 * 1024 * 1024;
    });
    socket
        .get_ref()
        .inner
        .set_nonblocking(true)
        .map_err(|_| err())?;
    Ok(())
}
fn require_painted(value: &Value) -> AppResult<()> {
    if value != &Value::Bool(true) {
        return Err(AppError::new(
            "WORKBUDDY_NOT_PAINTING",
            "WorkBuddy 暂停绘制。请显示窗口（不要最小化），停止滚动后刷新；不会自动切换你的会话。",
        ));
    }
    Ok(())
}
fn with_capture_paint<T>(
    cdp: &mut Cdp,
    capture: impl FnOnce(&mut Cdp) -> AppResult<T>,
) -> AppResult<T> {
    // Focus emulation is renderer-local: it does not foreground/restore a native window.
    // A new CDP connection owns this short lease; no persistent background polling.
    let hidden = cdp.evaluate("document.hidden")? == Value::Bool(true);
    let mut lease = PaintLease { cdp, reset: hidden };
    if hidden {
        lease.cdp.call(
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled":true}),
        )?;
    }
    let result = (|| {
        require_painted(&lease.cdp.evaluate(include_str!("live-paint-ready.js"))?)?;
        capture(lease.cdp)
    })();
    // Do not publish a frame if cleanup failed. Drop retries on any error/panic.
    lease.restore()?;
    result
}
struct PaintLease<'a> {
    cdp: &'a mut Cdp,
    reset: bool,
}
impl PaintLease<'_> {
    fn restore(&mut self) -> AppResult<()> {
        if self.reset {
            self.cdp.call(
                "Emulation.setFocusEmulationEnabled",
                json!({"enabled":false}),
            )?;
            self.reset = false;
        }
        Ok(())
    }
}
impl Drop for PaintLease<'_> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
impl Drop for Cdp {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        // Deadline expiry may prevent a close frame; never leave the peer waiting on our TCP end.
        let _ = self
            .socket
            .get_ref()
            .inner
            .shutdown(std::net::Shutdown::Both);
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionGeometry {
    pub region: region_theme::Region,
    pub rect: Rect,
    pub background: String,
    pub foreground: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Geometry {
    pub width: u32,
    pub height: u32,
    pub scene: String,
    pub regions: Vec<RegionGeometry>,
    pub mutation_epoch: u64,
    pub scroll_x: f64,
    pub scroll_y: f64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenderedPreview {
    pub id: String,
    pub trial_id: String,
    pub draft_id: String,
    pub sequence: u64,
    pub css_hash: String,
    pub target_id: String,
    pub width: u32,
    pub height: u32,
    pub scene: String,
    pub captured_at: String,
    pub workbuddy_version: Option<String>,
    pub image_data_url: String,
    pub capture_ms: u128,
}
#[derive(Clone)]
pub(crate) struct CapturedFrame {
    pub view: RenderedPreview,
    pub png: Vec<u8>,
}
pub(crate) fn capture(
    cdp: &mut Cdp,
    trial_id: &str,
    doc: &theme_library::ThemeDocument,
    runtime_id: &str,
    version: Option<String>,
) -> AppResult<CapturedFrame> {
    let start = Instant::now();
    cdp.verify(runtime_id, &doc.compiled.css)?;
    with_capture_paint(cdp, |cdp| {
        capture_painted(cdp, trial_id, doc, runtime_id, version, start)
    })
}
fn capture_painted(
    cdp: &mut Cdp,
    trial_id: &str,
    doc: &theme_library::ThemeDocument,
    runtime_id: &str,
    version: Option<String>,
    start: Instant,
) -> AppResult<CapturedFrame> {
    let geometry: Geometry =
        serde_json::from_value(cdp.evaluate(include_str!("live-dom-probe.js"))?)
            .map_err(|_| err())?;
    if geometry.width == 0
        || geometry.height == 0
        || u64::from(geometry.width) * u64::from(geometry.height) > 16_000_000
    {
        return Err(err());
    }
    let result=cdp.call("Page.captureScreenshot",json!({"format":"png","fromSurface":true,"captureBeyondViewport":false,"clip":{"x":0,"y":0,"width":geometry.width,"height":geometry.height,"scale":1}})).map_err(|_|AppError::new("WORKBUDDY_NOT_PAINTING","WorkBuddy 暂未返回画面。请显示 WorkBuddy 窗口（不要最小化），停止滚动后刷新；不会自动切换你的会话。"))?;
    let encoded = result["data"].as_str().ok_or_else(err)?;
    let png = decode_image_base64(
        encoded,
        18 * 1024 * 1024,
        "LIVE_CAPTURE_SIZE",
        "实机截图超过大小限制。",
    )?;
    // Capture and metadata must still describe the same renderer, viewport and theme.
    cdp.verify(runtime_id, &doc.compiled.css)?;
    let after: Geometry = serde_json::from_value(cdp.evaluate(include_str!("live-dom-probe.js"))?)
        .map_err(|_| err())?;
    if after != geometry {
        return Err(AppError::new(
            "CAPTURE_CHANGED",
            "截图期间页面发生变化，请停止滚动或输入后刷新。",
        ));
    }
    let view = RenderedPreview {
        id: uuid::Uuid::new_v4().to_string(),
        trial_id: trial_id.into(),
        draft_id: doc.draft_id.clone(),
        sequence: doc.edit_sequence,
        css_hash: css_hash(&doc.compiled.css),
        target_id: cdp.target_id.clone(),
        width: geometry.width,
        height: geometry.height,
        scene: geometry.scene.clone(),
        captured_at: now_timestamp(),
        workbuddy_version: version,
        image_data_url: format!("data:image/png;base64,{encoded}"),
        capture_ms: start.elapsed().as_millis(),
    };
    Ok(CapturedFrame { view, png })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occluded_capture_restores_emulation_on_success_and_every_failure() {
        for failure in ["none", "enable", "paint", "capture", "restore"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut socket = tungstenite::accept(stream).unwrap();
                let mut calls = Vec::new();
                while let Ok(Message::Text(message)) = socket.read() {
                    let r: Value = serde_json::from_str(&message).unwrap();
                    let method = r["method"].as_str().unwrap();
                    calls.push((method.to_string(), r["params"]["enabled"].clone()));
                    let is_enable = method == "Emulation.setFocusEmulationEnabled"
                        && r["params"]["enabled"] == true;
                    let is_restore = method == "Emulation.setFocusEmulationEnabled"
                        && r["params"]["enabled"] == false;
                    let fail = (failure == "enable" && is_enable)
                        || (failure == "restore" && is_restore)
                        || (failure == "capture" && method == "Page.captureScreenshot");
                    let result = if method == "Runtime.evaluate" {
                        json!({"result":{"value": calls.len() == 1 || failure != "paint"}})
                    } else {
                        json!({})
                    };
                    let reply = if fail {
                        json!({"id":r["id"],"error":{"code":-32000}})
                    } else {
                        json!({"id":r["id"],"result":result})
                    };
                    socket
                        .send(Message::Text(reply.to_string().into()))
                        .unwrap();
                }
                calls
            });
            let socket = handshake(
                TcpStream::connect(address).unwrap(),
                &format!("ws://{address}/fixture"),
                Duration::from_secs(1),
            )
            .unwrap();
            let mut cdp = Cdp {
                socket,
                next: 0,
                target_id: "fixture".into(),
            };
            let result = with_capture_paint(&mut cdp, |cdp| {
                cdp.call("Page.captureScreenshot", json!({}))
            });
            drop(cdp);
            let calls = server.join().unwrap();
            assert_eq!(calls.first().unwrap().0, "Runtime.evaluate");
            assert_eq!(
                calls.last().unwrap(),
                &(
                    "Emulation.setFocusEmulationEnabled".to_string(),
                    json!(false)
                ),
                "{failure}: override must be reset even if enabling failed"
            );
            assert_eq!(result.is_ok(), failure == "none", "{failure}");
            if matches!(failure, "enable" | "paint") {
                assert!(!calls.iter().any(|c| c.0 == "Page.captureScreenshot"));
            }
        }
    }
    #[test]
    fn normal_cdp_reply_and_expired_drop_close_the_local_transport() {
        for expired in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut socket = tungstenite::accept(stream).unwrap();
                let request: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":{"ok":true}})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                let ending = socket.read();
                assert!(
                    !matches!(ending, Err(tungstenite::Error::Io(e)) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)),
                    "dropping CDP must release the peer even after its deadline"
                );
            });
            let stream = TcpStream::connect(address).unwrap();
            let mut socket = handshake(
                stream,
                &format!("ws://{address}/fixture"),
                Duration::from_secs(1),
            )
            .unwrap();
            configure_socket(&mut socket).unwrap();
            let mut cdp = Cdp {
                socket,
                next: 0,
                target_id: "fixture".into(),
            };
            assert_eq!(
                cdp.call("Runtime.evaluate", json!({})).unwrap(),
                json!({"ok":true})
            );
            if expired {
                cdp.socket.get_mut().deadline = Instant::now();
            }
            drop(cdp);
            server.join().unwrap();
        }
    }
    #[test]
    fn expired_io_neither_reads_nor_writes_and_also_rejects_flush() {
        let mut stream = DeadlineIo {
            inner: Cursor::new(vec![1, 2, 3]),
            deadline: Instant::now(),
        };
        assert_eq!(
            stream.read(&mut [0; 1]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            stream.write(&[9]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(stream.flush().unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(stream.inner.position(), 0);
        assert_eq!(stream.inner.into_inner(), vec![1, 2, 3]);
    }
    #[test]
    fn continuously_ready_unfinished_frames_still_reach_the_io_deadline() {
        struct ReadyFragments {
            started: Instant,
            bytes: usize,
        }
        impl Read for ReadyFragments {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                // Deterministic no-WouldBlock peer: first an empty unfinished text frame,
                // then unlimited empty continuation frames. Stop after 400ms if unprotected.
                if self.started.elapsed() >= Duration::from_millis(400) {
                    return Err(io::ErrorKind::ConnectionReset.into());
                }
                let size = buffer.len().min(4096);
                for value in &mut buffer[..size] {
                    *value = if self.bytes == 0 { 0x01 } else { 0x00 };
                    self.bytes += 1;
                }
                Ok(size)
            }
        }
        impl Write for ReadyFragments {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                Ok(buffer.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let started = Instant::now();
        let stream = DeadlineIo {
            inner: ReadyFragments { started, bytes: 0 },
            deadline: started + Duration::from_millis(60),
        };
        let mut socket =
            WebSocket::from_raw_socket(stream, tungstenite::protocol::Role::Client, None);
        assert!(
            matches!(socket.read(), Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::TimedOut)
        );
        assert!(started.elapsed() < Duration::from_millis(300));
    }
    #[test]
    fn handshake_deadline_cannot_be_extended_by_slow_http_headers() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nX-Padding: ")
                .unwrap();
            for _ in 0..30 {
                std::thread::sleep(Duration::from_millis(20));
                if stream.write_all(b"x").is_err() {
                    break;
                }
            }
        });
        let stream = TcpStream::connect(address).unwrap();
        let started = Instant::now();
        let result = handshake(
            stream,
            &format!("ws://{address}/fixture"),
            Duration::from_millis(60),
        );
        let elapsed = started.elapsed();
        server.join().unwrap();
        assert!(result.is_err());
        assert!(
            elapsed < Duration::from_millis(300),
            "trickling headers exceeded the handshake budget: {elapsed:?}"
        );
    }
    #[test]
    fn hidden_or_unpainted_pages_never_report_a_fresh_frame() {
        assert!(require_painted(&json!(true)).is_ok());
        for value in [json!(false), Value::Null, json!({"value":true})] {
            assert_eq!(
                require_painted(&value).unwrap_err().code,
                "WORKBUDDY_NOT_PAINTING"
            );
        }
    }
    #[test]
    fn cdp_deadline_covers_unrelated_events_and_fragmented_messages() {
        use tungstenite::protocol::{
            frame::{
                coding::{Data, OpCode},
                Frame,
            },
            Role,
        };
        for fragments in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let client_stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (server_stream, _) = listener.accept().unwrap();
            let server = std::thread::spawn(move || {
                let mut socket = WebSocket::from_raw_socket(server_stream, Role::Server, None);
                socket.read().unwrap();
                for i in 0..30 {
                    std::thread::sleep(Duration::from_millis(20));
                    let message = if fragments {
                        Message::Frame(Frame::message(
                            vec![b'x'],
                            OpCode::Data(if i == 0 { Data::Text } else { Data::Continue }),
                            false,
                        ))
                    } else {
                        Message::Text("{\"method\":\"unrelated\"}".into())
                    };
                    if socket.send(message).is_err() {
                        break;
                    }
                }
            });
            let mut socket = WebSocket::from_raw_socket(
                DeadlineIo {
                    inner: client_stream,
                    deadline: Instant::now() + Duration::from_secs(1),
                },
                Role::Client,
                None,
            );
            configure_socket(&mut socket).unwrap();
            let mut cdp = Cdp {
                socket,
                next: 0,
                target_id: "fixture".into(),
            };
            let start = Instant::now();
            assert!(cdp
                .call_with_timeout(
                    "Page.captureScreenshot",
                    json!({}),
                    Duration::from_millis(90)
                )
                .is_err());
            assert!(
                start.elapsed() < Duration::from_millis(450),
                "a partial frame cannot extend the command deadline"
            );
            drop(cdp);
            server.join().unwrap();
        }
    }
    #[test]
    fn screenshot_hash_changes_with_css() {
        assert_ne!(css_hash("a"), css_hash("b"));
    }
}
