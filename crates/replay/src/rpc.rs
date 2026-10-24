//! A deliberately small safe-RPC boundary. No storage values or execution RPCs are allowed.
use crate::{Error, Result};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

const ALLOWED: &[&str] = &[
    "chain_getBlock",
    "chain_getHeader",
    "chain_getBlockHash",
    "state_getRuntimeVersion",
    "state_getReadProof",
    "state_getChildReadProof",
    "state_getMetadata",
];
type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

#[derive(Clone)]
pub struct Rpc {
    transport: Arc<Transport>,
    calls: Arc<AtomicUsize>,
}
enum Transport {
    Http {
        url: String,
        client: reqwest::blocking::Client,
    },
    Ws(Box<Mutex<Socket>>),
}

impl Rpc {
    /// Connects to WS/WSS or HTTP(S). Calls are bounded by a 90 second I/O timeout.
    pub fn new(url: &str) -> Result<Self> {
        let transport = if url.starts_with("ws://") || url.starts_with("wss://") {
            let (mut socket, _) =
                tungstenite::connect(url).map_err(|e| Error::Rpc(format!("connect {url}: {e}")))?;
            let stream = match socket.get_mut() {
                MaybeTlsStream::Plain(s) => s,
                MaybeTlsStream::Rustls(s) => &mut s.sock,
                _ => return Err(Error::Rpc("unsupported WebSocket TLS transport".into())),
            };
            stream.set_read_timeout(Some(Duration::from_secs(90)))?;
            stream.set_write_timeout(Some(Duration::from_secs(90)))?;
            Transport::Ws(Box::new(Mutex::new(socket)))
        } else if url.starts_with("https://") || url.starts_with("http://") {
            Transport::Http {
                url: url.into(),
                client: reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(90))
                    .build()
                    .map_err(|e| Error::Rpc(e.to_string()))?,
            }
        } else {
            return Err(Error::Rpc(
                "expected ws://, wss://, http:// or https:// URL".into(),
            ));
        };
        Ok(Self {
            transport: Arc::new(transport),
            calls: Default::default(),
        })
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        if !ALLOWED.contains(&method) {
            return Err(Error::MethodNotAllowed(method.into()));
        }
        // Metadata is auxiliary decoding data. Never allow an unpinned latest-state request.
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
        let id = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let request = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
        let body: Value = match &*self.transport {
            Transport::Http { url, client } => client
                .post(url)
                .json(&request)
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .and_then(|r| r.json())
                .map_err(|e| Error::Rpc(format!("{method}: {e}")))?,
            Transport::Ws(socket) => {
                let mut socket = socket.lock();
                socket
                    .send(Message::Text(request.to_string().into()))
                    .map_err(|e| Error::Rpc(format!("{method}: {e}")))?;
                loop {
                    match socket
                        .read()
                        .map_err(|e| Error::Rpc(format!("{method}: {e}")))?
                    {
                        Message::Text(s) => {
                            let value: Value = serde_json::from_str(&s)?;
                            if value["id"] == id {
                                break value;
                            }
                            return Err(Error::Rpc(format!("{method}: unexpected response ID")));
                        }
                        Message::Ping(_) | Message::Pong(_) => continue,
                        Message::Close(_) => {
                            return Err(Error::Rpc(format!("{method}: connection closed")));
                        }
                        _ => {
                            return Err(Error::Rpc(format!(
                                "{method}: unexpected WebSocket message"
                            )));
                        }
                    }
                }
            }
        };
        if body["id"] != id {
            return Err(Error::Rpc(format!("{method}: response ID mismatch")));
        }
        if let Some(error) = body.get("error").filter(|e| !e.is_null()) {
            return Err(Error::Rpc(format!(
                "{method}: {error}; parent state may be pruned (use an archive endpoint)"
            )));
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| Error::Rpc(format!("{method}: no result")))
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
