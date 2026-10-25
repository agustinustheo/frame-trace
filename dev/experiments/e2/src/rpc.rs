use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Rpc {
    url: String,
    client: reqwest::blocking::Client,
    pub calls: Arc<Mutex<Vec<Value>>>,
}

impl Rpc {
    pub fn new(url: String) -> Result<Self> {
        Ok(Self {
            url,
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(90))
                .build()?,
            calls: Default::default(),
        })
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        // Keep the replay source incapable of using unsafe or node-local APIs.
        if !matches!(
            method,
            "chain_getBlock"
                | "chain_getHeader"
                | "state_getStorage"
                | "state_getRuntimeVersion"
                | "state_getReadProof"
        ) {
            bail!("RPC method outside the experiment's safe allowlist: {method}");
        }
        let id = self.calls.lock().len() + 1;
        let start = Instant::now();
        let response = self
            .client
            .post(&self.url)
            .json(&json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params
            }))
            .send()
            .with_context(|| format!("{method} HTTP request"))?;
        let status = response.status();
        let bytes = response.bytes()?;
        let body: Value = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "{method}: HTTP {status}, {}",
                String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
            )
        })?;
        self.calls.lock().push(json!({ "method": method, "params": params,
            "elapsed_ms": start.elapsed().as_secs_f64() * 1000.,
            "response_bytes": bytes.len(), "http_status": status.as_u16(), "error": body.get("error") }));
        if std::env::var_os("E2_RPC_PROGRESS").is_some() {
            eprintln!(
                "RPC {id}: {method}, {:.1} ms, {} bytes",
                start.elapsed().as_secs_f64() * 1000.,
                bytes.len()
            );
        }
        if let Some(error) = body.get("error") {
            bail!("{method}: {error}");
        }
        body.get("result")
            .cloned()
            .context("RPC response has no result")
    }

    pub fn counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for call in self.calls.lock().iter() {
            *counts
                .entry(call["method"].as_str().unwrap().to_string())
                .or_default() += 1;
        }
        counts
    }
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    Ok(hex::decode(s.strip_prefix("0x").unwrap_or(s))?)
}
pub fn hex0(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}
