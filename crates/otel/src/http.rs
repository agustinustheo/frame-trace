//! Inspect OTLP responses before the upstream exporter discards partial success.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
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
}

impl HttpExporter {
    pub(crate) fn new(endpoint: String) -> Result<Self, crate::Error> {
        // Match the upstream client's timeout resolution, including invalid-value fallback.
        let timeout = [
            "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
            "OTEL_EXPORTER_OTLP_TIMEOUT",
        ]
        .into_iter()
        .find_map(|name| std::env::var(name).ok()?.parse().ok())
        .map_or(
            opentelemetry_otlp::OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT,
            Duration::from_millis,
        );
        // Like upstream, build the blocking client outside any caller's async runtime.
        let client = std::thread::Builder::new()
            .spawn(move || {
                reqwest::blocking::Client::builder()
                    .timeout(timeout)
                    .build()
            })
            .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?
            .join()
            .map_err(|_| {
                ExporterBuildError::InternalFailure("HTTP client builder panicked".into())
            })?
            .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?;
        let rejections = Arc::new(Mutex::new(Rejections::default()));
        let inner = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .with_timeout(timeout)
            .with_http_client(CheckedClient {
                client,
                rejections: rejections.clone(),
            })
            .build()?;
        Ok(Self { inner, rejections })
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
    client: reqwest::blocking::Client,
    rejections: Arc<Mutex<Rejections>>,
}

#[async_trait]
impl HttpClient for CheckedClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let response = self.client.send_bytes(request).await?;
        if response.status().is_success()
            && let Some(partial) = partial_success(&response)
            && partial.rejected_spans > 0
        {
            let mut rejections = self
                .rejections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if rejections.rejected == 0 {
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
