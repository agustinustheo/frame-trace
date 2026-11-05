//! Inspect OTLP responses before the upstream exporter discards partial success.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{ExporterBuildError, WithExportConfig, WithHttpConfig};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceResponse,
};
use opentelemetry_sdk::{
    Resource,
    error::{OTelSdkError, OTelSdkResult},
    trace::{SpanData, SpanExporter},
};
use prost::Message;

#[derive(Debug, Default)]
pub(crate) struct Rejections {
    pub rejected: u64,
    pub message: String,
}

/// OTLP/HTTP exporter that treats partial rejection as an export failure.
#[derive(Debug)]
pub struct HttpExporter {
    inner: opentelemetry_otlp::SpanExporter,
    pub(crate) rejections: Arc<Mutex<Rejections>>,
    // Ends every request of the batch [`HttpExporter::send`] is sending, retries included.
    deadline: Arc<Mutex<Option<Instant>>>,
    timeout: Duration,
}

impl HttpExporter {
    pub(crate) fn new(endpoint: String, timeout: Option<Duration>) -> Result<Self, crate::Error> {
        // Match the upstream client's timeout resolution, including invalid-value fallback.
        let timeout = timeout.unwrap_or_else(|| {
            [
                "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
                "OTEL_EXPORTER_OTLP_TIMEOUT",
            ]
            .into_iter()
            .find_map(|name| std::env::var(name).ok()?.parse().ok())
            .map_or(
                opentelemetry_otlp::OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT,
                Duration::from_millis,
            )
        });
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?;
        // Requests run on this small runtime, on the caller's thread, under one deadline.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?;
        let rejections = Arc::new(Mutex::new(Rejections::default()));
        let deadline = Arc::new(Mutex::new(None));
        let inner = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .with_timeout(timeout)
            .with_http_client(CheckedClient {
                client,
                runtime,
                timeout,
                deadline: deadline.clone(),
                rejections: rejections.clone(),
            })
            .build()?;
        Ok(Self {
            inner,
            rejections,
            deadline,
            timeout,
        })
    }
}

impl HttpExporter {
    /// Sends one batch on this thread and returns how many of its spans the receiver rejected,
    /// with the receiver's rejection message for this batch. The exporter's own retries share
    /// one deadline of the export timeout, which covers connecting, the response and its body.
    /// Nothing is left running when it returns.
    pub(crate) fn send(&self, batch: Vec<SpanData>) -> Result<(u64, String), OTelSdkError> {
        let before = {
            let mut rejections = self
                .rejections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            rejections.message.clear();
            rejections.rejected
        };
        *self.deadline.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Instant::now() + self.timeout);
        let result = futures_executor::block_on(self.inner.export(batch));
        *self.deadline.lock().unwrap_or_else(PoisonError::into_inner) = None;
        result?;
        let rejections = self
            .rejections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok((rejections.rejected - before, rejections.message.clone()))
    }
}

impl SpanExporter for HttpExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let total = batch.len();
        let before = self
            .rejections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rejected;
        self.inner.export(batch).await?;
        let rejections = self
            .rejections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let rejected = rejections.rejected - before;
        if rejected > 0 {
            return Err(OTelSdkError::InternalFailure(format!(
                "{rejected} of {total} spans rejected by the receiver: {}",
                rejections.message
            )));
        }
        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }
}

#[derive(Debug)]
struct CheckedClient {
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
    timeout: Duration,
    deadline: Arc<Mutex<Option<Instant>>>,
    rejections: Arc<Mutex<Rejections>>,
}

/// Largest response body read from the receiver.
const MAX_RESPONSE_BYTES: usize = 4 << 20;

impl CheckedClient {
    /// One request with an absolute deadline. reqwest's blocking client restarts its timeout
    /// for every read, so a receiver sending its body slowly could hold it much longer.
    fn exchange(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let deadline = self
            .deadline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or_else(|| Instant::now() + self.timeout);
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("export deadline exceeded".into());
        }
        let request = reqwest::Request::try_from(request)?;
        self.runtime.block_on(async {
            tokio::time::timeout(left, async {
                let mut response = self.client.execute(request).await?;
                let mut builder = Response::builder().status(response.status());
                for (name, value) in response.headers() {
                    builder = builder.header(name, value);
                }
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    body.extend_from_slice(&chunk);
                    if body.len() > MAX_RESPONSE_BYTES {
                        return Err::<_, HttpError>("response body too large".into());
                    }
                }
                Ok(builder.body(Bytes::from(body))?)
            })
            .await
            .unwrap_or_else(|_| Err("export deadline exceeded".into()))
        })
    }
}

#[async_trait]
impl HttpClient for CheckedClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let response = self.exchange(request)?;
        if response.status().is_success()
            && let Some(partial) = partial_success(&response)
            && partial.rejected_spans > 0
        {
            let mut rejections = self
                .rejections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // The first message since the batch began; [`HttpExporter::send`] clears it.
            if rejections.message.is_empty() {
                rejections.message = partial.error_message;
            }
            rejections.rejected += partial.rejected_spans as u64;
        }
        // Leave the HTTP response intact: partial success must never trigger a retry.
        Ok(response)
    }
}

fn partial_success(response: &Response<Bytes>) -> Option<ExportTracePartialSuccess> {
    let json = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("application/json")
        });
    if !json {
        return ExportTraceServiceResponse::decode(response.body().as_ref())
            .ok()?
            .partial_success;
    }
    // OTLP JSON represents int64 as a decimal string; accept numbers as well.
    let response: serde_json::Value = serde_json::from_slice(response.body()).ok()?;
    let partial = response
        .get("partialSuccess")
        .or_else(|| response.get("partial_success"))?;
    let rejected = partial
        .get("rejectedSpans")
        .or_else(|| partial.get("rejected_spans"));
    let rejected_spans = match rejected {
        None => 0,
        Some(value) => value.as_i64().or_else(|| value.as_str()?.parse().ok())?,
    };
    Some(ExportTracePartialSuccess {
        rejected_spans,
        error_message: partial
            .get("errorMessage")
            .or_else(|| partial.get("error_message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}
