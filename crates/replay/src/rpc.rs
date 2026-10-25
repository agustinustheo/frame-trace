//! A small safe-RPC boundary, with cancellable deadlines, pooled connections and retries.
use crate::{Error, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{runtime::Runtime, sync::Mutex};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
pub use tokio_util::sync::CancellationToken;

const ALLOWED: &[&str] = &[
    "chain_getFinalizedHead",
    "chain_getBlock",
    "chain_getHeader",
    "chain_getBlockHash",
    "state_getRuntimeVersion",
    "state_getReadProof",
    "state_getChildReadProof",
    "state_getMetadata",
    "state_call",
];
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone)]
pub struct Rpc {
    inner: Arc<Inner>,
    // Per-client counters; forked clients share transport but isolate measured-run accounting.
    calls: Arc<AtomicUsize>,
}
struct Inner {
    url: String,
    http: reqwest::Client,
    tls: Arc<rustls::ClientConfig>,
    sockets: Vec<Mutex<Option<Socket>>>,
    next: AtomicUsize,
    limit: tokio::sync::Semaphore,
    cancel: CancellationToken,
    deadline: Duration,
}

fn runtime() -> Result<&'static Runtime> {
    static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Error::Rpc(e.clone()))
}
impl Rpc {
    /// Lazy connections. DNS, TCP, TLS and WS handshake share a ten-second connect deadline;
    /// the entire logical call (including retries and pool wait) has a thirty-second deadline.
    pub fn new(url: &str) -> Result<Self> {
        Self::with_options(url, 8, Duration::from_secs(30), CancellationToken::new())
    }
    pub fn with_options(
        url: &str,
        parallelism: usize,
        deadline: Duration,
        cancel: CancellationToken,
    ) -> Result<Self> {
        if !["ws://", "wss://", "http://", "https://"]
            .iter()
            .any(|s| url.starts_with(s))
        {
            return Err(Error::Rpc(
                "expected ws://, wss://, http:// or https:// URL".into(),
            ));
        }
        if !(1..=64).contains(&parallelism) {
            return Err(Error::Invalid(
                "proof parallelism must be between 1 and 64".into(),
            ));
        }
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Rpc(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls.clone())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Rpc(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                url: url.into(),
                http,
                tls: Arc::new(tls),
                sockets: (0..parallelism).map(|_| Mutex::new(None)).collect(),
                next: AtomicUsize::new(0),
                limit: tokio::sync::Semaphore::new(parallelism),
                cancel,
                deadline,
            }),
            calls: Default::default(),
        })
    }
    pub fn fork(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            calls: Default::default(),
        }
    }
    pub fn parallelism(&self) -> usize {
        self.inner.sockets.len()
    }
    pub fn cancelled(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        if !ALLOWED.contains(&method) {
            return Err(Error::MethodNotAllowed(method.into()));
        }
        if method == "state_getMetadata"
            && (params.as_array().is_none_or(|a| a.len() != 1)
                || params[0]
                    .as_str()
                    .is_none_or(|s| s.parse::<sp_core::H256>().is_err()))
        {
            return Err(Error::Rpc(
                "metadata must be fetched at an explicit parent/block hash".into(),
            ));
        }
        if method == "state_call"
            && (params.as_array().is_none_or(|a| a.len() != 3)
                || params[0].as_str().is_none_or(str::is_empty)
                || params[1].as_str().is_none_or(|s| unhex(s).is_err())
                || params[2]
                    .as_str()
                    .is_none_or(|s| s.parse::<sp_core::H256>().is_err()))
        {
            return Err(Error::Rpc(
                "state_call requires method, SCALE hex and explicit state hash".into(),
            ));
        }
        runtime()?.block_on(async {
            tokio::select! {
                biased;
                _ = self.inner.cancel.cancelled() => Err(Error::Cancelled),
                result = tokio::time::timeout(self.inner.deadline, self.retry(method, params)) =>
                    result.unwrap_or_else(|_| Err(Error::TransientRpc(format!("{method}: overall request deadline exceeded")))),
            }
        })
    }
    async fn retry(&self, method: &str, params: Value) -> Result<Value> {
        for attempt in 0..4 {
            let id = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let request = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
            let attempt_deadline = (self.inner.deadline / 4).min(Duration::from_secs(10));
            let response = tokio::time::timeout(attempt_deadline, self.once(&request))
                .await
                .unwrap_or_else(|_| Err(("attempt deadline exceeded".into(), true)));
            match response {
                Ok(body) => {
                    if body["jsonrpc"] != "2.0" || body["id"] != id {
                        return Err(Error::Rpc(format!(
                            "{method}: malformed response or response ID mismatch"
                        )));
                    }
                    if let Some(error) = body.get("error").filter(|e| !e.is_null()) {
                        let text = error.to_string();
                        if attempt < 3
                            && (error["code"] == 429
                                || text.to_lowercase().contains("too many requests"))
                        {
                            self.backoff(attempt, id).await;
                            continue;
                        }
                        let lower = text.to_lowercase();
                        let message = format!("{method}: {error}");
                        if lower.contains("state already discarded")
                            || lower.contains("state is not available")
                            || lower.contains("pruned")
                        {
                            return Err(Error::Unavailable(format!(
                                "{message}; parent state may be pruned (use an archive endpoint)"
                            )));
                        }
                        if method == "state_getMetadata" && error["code"] == -32601 {
                            return Err(Error::Unavailable(message));
                        }
                        if error["code"] == 429 || lower.contains("too many requests") {
                            return Err(Error::TransientRpc(message));
                        }
                        return Err(Error::RpcResponse {
                            method: method.into(),
                            code: error["code"].as_i64().unwrap_or(-32603),
                            message: error["message"].as_str().unwrap_or(&text).into(),
                        });
                    }
                    return body
                        .get("result")
                        .cloned()
                        .ok_or_else(|| Error::Rpc(format!("{method}: no result")));
                }
                Err((message, transient)) => {
                    if !transient || attempt == 3 {
                        return Err(if transient {
                            Error::TransientRpc(format!("{method}: {message}"))
                        } else {
                            Error::Rpc(format!("{method}: {message}"))
                        });
                    }
                    self.backoff(attempt, id).await;
                }
            }
        }
        unreachable!("bounded retry loop returns on last attempt")
    }
    async fn backoff(&self, attempt: u32, id: usize) {
        // Capped exponential delay, with per-request and wall-clock jitter to avoid herding.
        let jitter = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |t| (t.subsec_nanos() as u64 ^ id as u64) % 101);
        tokio::time::sleep(Duration::from_millis(
            (100u64 << attempt).min(2000) + jitter,
        ))
        .await;
    }
    async fn once(&self, request: &Value) -> std::result::Result<Value, (String, bool)> {
        let _permit = self
            .inner
            .limit
            .acquire()
            .await
            .map_err(|e| (e.to_string(), false))?;
        if self.inner.url.starts_with("http") {
            let response = self
                .inner
                .http
                .post(&self.inner.url)
                .json(request)
                .send()
                .await
                .map_err(|e| {
                    (
                        e.to_string(),
                        e.is_timeout() || e.is_connect() || e.is_request(),
                    )
                })?;
            let status = response.status();
            if !status.is_success() {
                return Err((
                    format!("HTTP {status}"),
                    status.as_u16() == 429 || status.is_server_error(),
                ));
            }
            return response
                .json()
                .await
                .map_err(|e| (format!("malformed response: {e}"), false));
        }
        let index = self.inner.next.fetch_add(1, Ordering::Relaxed) % self.parallelism();
        let mut slot = match self
            .inner
            .sockets
            .iter()
            .find_map(|slot| slot.try_lock().ok())
        {
            Some(slot) => slot,
            None => self.inner.sockets[index].lock().await,
        };
        // Take ownership: dropping a timed-out/cancelled future closes the socket, so its
        // outstanding response can never be consumed by the next call.
        let mut socket = match slot.take() {
            Some(socket) => socket,
            None => {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    tokio_tungstenite::connect_async_tls_with_config(
                        self.inner.url.as_str(),
                        None,
                        true,
                        Some(tokio_tungstenite::Connector::Rustls(self.inner.tls.clone())),
                    ),
                )
                .await
                .map_err(|_| {
                    (
                        format!(
                            "connect {}: TCP/TLS/WebSocket handshake deadline exceeded",
                            self.inner.url
                        ),
                        true,
                    )
                })?
                .map_err(|e| (format!("connect {}: {e}", self.inner.url), true))?
                .0
            }
        };
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|e| (e.to_string(), true))?;
        while let Some(message) = socket.next().await {
            match message.map_err(|e| (e.to_string(), true))? {
                Message::Text(text) => {
                    let value = serde_json::from_str(&text)
                        .map_err(|e| (format!("malformed response: {e}"), false))?;
                    *slot = Some(socket);
                    return Ok(value);
                }
                Message::Ping(_) | Message::Pong(_) => {
                    socket.flush().await.map_err(|e| (e.to_string(), true))?;
                }
                Message::Close(_) => return Err(("connection closed".into(), true)),
                _ => return Err(("unexpected WebSocket message".into(), false)),
            }
        }
        Err(("connection dropped".into(), true))
    }
    pub fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
pub fn unhex(s: &str) -> Result<Vec<u8>> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s))
        .map_err(|e| Error::Invalid(format!("invalid hex: {e}")))
}
pub fn hex0(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread,
    };
    fn responses(replies: Vec<(u16, &'static str)>) -> (Rpc, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(15)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut bytes = vec![0; len];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                if status == 0 {
                    let _ = reader.read_to_end(&mut Vec::new());
                    continue;
                }
                let status = if status == 0 { 200 } else { status };
                let body = body.replace("$ID", &request["id"].to_string());
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (Rpc::new(&url).unwrap(), worker)
    }
    #[test]
    fn mock_responses_stops_at_request_eof() {
        let (rpc, worker) = responses(vec![(200, "unused")]);
        let address = rpc.inner.url.strip_prefix("http://").unwrap();
        drop(std::net::TcpStream::connect(address).unwrap());
        worker.join().unwrap();
    }
    #[test]
    fn exhausted_rate_limits_remain_transient_and_cancellation_is_typed() {
        let (rpc, worker) = responses(vec![(429, "busy"); 4]);
        assert!(
            rpc.call("chain_getHeader", json!([]))
                .unwrap_err()
                .is_transient()
        );
        assert_eq!(rpc.count(), 4);
        worker.join().unwrap();
        rpc.inner.cancel.cancel();
        assert!(matches!(
            rpc.call("chain_getHeader", json!([])),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn http_429_and_json_rpc_rate_limits_are_retried() {
        let (rpc, worker) = responses(vec![
            (429, "rate limited"),
            (
                200,
                r#"{"jsonrpc":"2.0","id":$ID,"error":{"code":429,"message":"busy"}}"#,
            ),
            (200, r#"{"jsonrpc":"2.0","id":$ID,"result":"ok"}"#),
        ]);
        assert_eq!(rpc.call("chain_getHeader", json!([])).unwrap(), "ok");
        assert_eq!(rpc.count(), 3);
        worker.join().unwrap();
    }
    #[test]
    fn request_timeouts_are_retried_within_the_overall_deadline() {
        let (initial, worker) = responses(vec![
            (0, r#"{"jsonrpc":"2.0","id":$ID,"result":"late"}"#),
            (200, r#"{"jsonrpc":"2.0","id":$ID,"result":"ok"}"#),
        ]);
        let rpc = Rpc::with_options(
            &initial.inner.url,
            1,
            Duration::from_secs(8),
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(rpc.call("chain_getHeader", json!([])).unwrap(), "ok");
        assert_eq!(rpc.count(), 2);
        worker.join().unwrap();
    }
    #[test]
    fn json_rpc_errors_only_describe_pruning_when_specific() {
        for (message, pruning) in [
            ("Invalid params", false),
            ("Method not found", false),
            ("State already discarded", true),
        ] {
            let body = format!(
                r#"{{"jsonrpc":"2.0","id":$ID,"error":{{"code":-32000,"message":"{message}"}}}}"#
            );
            // The loopback server owns these static test strings for its short lifetime.
            let (rpc, worker) = responses(vec![(200, Box::leak(body.into_boxed_str()))]);
            let error = rpc
                .call("chain_getHeader", json!([]))
                .unwrap_err()
                .to_string();
            assert_eq!(
                error.contains("parent state may be pruned"),
                pruning,
                "{error}"
            );
            assert!(error.contains(message));
            assert_eq!(rpc.count(), 1);
            worker.join().unwrap();
        }
    }
    #[test]
    fn malformed_json_envelopes_and_null_blocks_have_specific_errors() {
        for body in [
            "not json",
            r#"{"jsonrpc":"2.0","id":999,"result":null}"#,
            r#"{"jsonrpc":"2.0","id":$ID}"#,
            r#"{"jsonrpc":"1.0","id":$ID,"result":null}"#,
        ] {
            let (rpc, worker) = responses(vec![(200, body)]);
            assert!(rpc.call("chain_getHeader", json!([])).is_err());
            worker.join().unwrap();
        }
        for selector in [
            "1000",
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "best",
            "finalized",
        ] {
            let (rpc, worker) =
                responses(vec![(200, r#"{"jsonrpc":"2.0","id":$ID,"result":null}"#)]);
            let error = crate::online(&rpc, selector).unwrap_err().to_string();
            assert!(
                error.contains(&format!("block {selector} not found")),
                "{error}"
            );
            worker.join().unwrap();
        }
    }
    #[test]
    fn dropped_websocket_connection_reconnects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            for success in [false, true] {
                let mut socket = tungstenite::accept(listener.accept().unwrap().0).unwrap();
                let request: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                if success {
                    socket
                        .send(tungstenite::Message::Text(
                            json!({"jsonrpc":"2.0", "id":request["id"], "result":7})
                                .to_string()
                                .into(),
                        ))
                        .unwrap();
                }
            }
        });
        let rpc = Rpc::new(&url).unwrap();
        assert_eq!(rpc.call("chain_getHeader", json!([])).unwrap(), 7);
        assert_eq!(rpc.count(), 2);
        worker.join().unwrap();
    }
}
