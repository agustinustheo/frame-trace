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
        Self::with_client(endpoint, timeout, reqwest::Client::builder())
    }

    fn with_client(
        endpoint: String,
        timeout: Option<Duration>,
        client: reqwest::ClientBuilder,
    ) -> Result<Self, crate::Error> {
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
        let client = client
            .timeout(timeout)
            .build()
            .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?;
        let jobs = transport(client)?;
        let rejections = Arc::new(Mutex::new(Rejections::default()));
        let deadline = Arc::new(Mutex::new(None));
        let inner = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .with_timeout(timeout)
            .with_http_client(CheckedClient {
                jobs,
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
        futures_executor::block_on(self.export_batch(batch))
    }

    /// The one scope of a batch, for both entry points: starts its deadline, clears the
    /// previous batch's rejection message, and returns this batch's rejections.
    async fn export_batch(&self, batch: Vec<SpanData>) -> Result<(u64, String), OTelSdkError> {
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
        let result = self.inner.export(batch).await;
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
        let (rejected, message) = self.export_batch(batch).await?;
        if rejected > 0 {
            return Err(OTelSdkError::InternalFailure(format!(
                "{rejected} of {total} spans rejected by the receiver: {message}"
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
    jobs: tokio::sync::mpsc::UnboundedSender<Job>,
    timeout: Duration,
    deadline: Arc<Mutex<Option<Instant>>>,
    rejections: Arc<Mutex<Rejections>>,
}

/// One request for the transport thread, and where to send its answer.
#[derive(Debug)]
struct Job {
    request: reqwest::Request,
    deadline: Instant,
    answer: tokio::sync::oneshot::Sender<Result<Response<Bytes>, HttpError>>,
}

/// Largest response body read from the receiver.
const MAX_RESPONSE_BYTES: usize = 4 << 20;

/// Starts the thread that owns the HTTP runtime. Callers on any thread, or inside their own
/// async runtime, only send it jobs and await the answers: they never start, enter or drop a
/// runtime. When the last sender is dropped, the thread ends without waiting for blocking work
/// such as a slow DNS lookup.
fn transport(
    client: reqwest::Client,
) -> Result<tokio::sync::mpsc::UnboundedSender<Job>, crate::Error> {
    let (jobs, mut queue) = tokio::sync::mpsc::unbounded_channel::<Job>();
    let (ready, started) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("otlp-http".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => {
                    let _ = ready.send(Ok(()));
                    runtime
                }
                Err(error) => {
                    let _ = ready.send(Err(error.to_string()));
                    return;
                }
            };
            runtime.block_on(async {
                while let Some(job) = queue.recv().await {
                    // A job that waited past its deadline is refused, so work the caller has
                    // given up on stays bounded even after this thread was held up.
                    let left = job.deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        let _ = job.answer.send(Err("export deadline exceeded".into()));
                        continue;
                    }
                    let client = client.clone();
                    tokio::spawn(async move {
                        let result = tokio::time::timeout(left, exchange(&client, job.request))
                            .await
                            .unwrap_or_else(|_| Err("export deadline exceeded".into()));
                        let _ = job.answer.send(result);
                    });
                }
            });
            runtime.shutdown_background();
        })
        .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?;
    started
        .recv()
        .map_err(|e| ExporterBuildError::InternalFailure(e.to_string()))?
        .map_err(ExporterBuildError::InternalFailure)?;
    Ok(jobs)
}

/// One request, reading the whole response body. The caller bounds it with the deadline:
/// reqwest's blocking client restarts its timeout for every read, so a receiver sending its
/// body slowly could hold it much longer.
async fn exchange(
    client: &reqwest::Client,
    request: reqwest::Request,
) -> Result<Response<Bytes>, HttpError> {
    let mut response = client.execute(request).await?;
    let mut builder = Response::builder().status(response.status());
    for (name, value) in response.headers() {
        builder = builder.header(name, value);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_RESPONSE_BYTES {
            return Err("response body too large".into());
        }
    }
    Ok(builder.body(Bytes::from(body))?)
}

impl CheckedClient {
    async fn exchange(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let deadline = self
            .deadline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or_else(|| Instant::now() + self.timeout);
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("export deadline exceeded".into());
        }
        let (answer, mut answered) = tokio::sync::oneshot::channel();
        self.jobs
            .send(Job {
                request: reqwest::Request::try_from(request)?,
                deadline,
                answer,
            })
            .map_err(|_| "OTLP transport thread stopped")?;
        // Waits without blocking the caller's thread, on any executor. The deadline ends the
        // wait even if the transport thread is stuck.
        let mut timer = Deadline::new(left);
        std::future::poll_fn(|cx| {
            use std::{future::Future, pin::Pin, task::Poll};
            if let Poll::Ready(result) = Pin::new(&mut answered).poll(cx) {
                return Poll::Ready(
                    result.unwrap_or_else(|_| Err("OTLP transport thread stopped".into())),
                );
            }
            match Pin::new(&mut timer).poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err("export deadline exceeded".into())),
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}

/// A future that completes after a delay, on any executor: a helper thread wakes it, so it
/// needs no runtime timer. Dropping it ends the thread at once.
struct Deadline {
    state: Arc<Mutex<(bool, Option<std::task::Waker>)>>,
    // Dropped with the future, which wakes the helper thread so it exits.
    _stop: std::sync::mpsc::Sender<()>,
}

impl Deadline {
    fn new(after: Duration) -> Self {
        let state = Arc::new(Mutex::new((false, None::<std::task::Waker>)));
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let shared = state.clone();
        let fire = move || {
            let waker = {
                let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
                state.0 = true;
                state.1.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        };
        // If the helper thread cannot start, the transport's own timeout still answers by the
        // deadline.
        let _ = std::thread::Builder::new()
            .name("otlp-deadline".into())
            .spawn(move || {
                if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(after)
                {
                    fire();
                }
            });
        Self { state, _stop: stop }
    }
}

impl std::future::Future for Deadline {
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.0 {
            return std::task::Poll::Ready(());
        }
        state.1 = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

#[async_trait]
impl HttpClient for CheckedClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let response = self.exchange(request).await?;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Resolves nothing, after blocking a thread of the runtime's blocking pool for 3 s, as a
    /// slow system resolver does.
    struct SlowResolver;
    impl reqwest::dns::Resolve for SlowResolver {
        fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
            Box::pin(async {
                let _ = tokio::task::spawn_blocking(|| {
                    std::thread::sleep(Duration::from_secs(3));
                })
                .await;
                Err("not found".into())
            })
        }
    }

    /// Blocks the transport's own runtime thread for 2 s, as a resolver that blocks would.
    struct WedgingResolver;
    impl reqwest::dns::Resolve for WedgingResolver {
        fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
            std::thread::sleep(Duration::from_secs(2));
            Box::pin(async { Err("not found".into()) })
        }
    }

    #[test]
    fn a_wedged_transport_thread_cannot_hold_the_caller_past_the_deadline() {
        let exporter = HttpExporter::with_client(
            "http://wedged.invalid:4318/v1/traces".into(),
            Some(Duration::from_millis(100)),
            reqwest::Client::builder().dns_resolver(Arc::new(WedgingResolver)),
        )
        .unwrap();
        let start = Instant::now();
        assert!(exporter.send(Vec::new()).is_err());
        let took = start.elapsed();
        assert!(took < Duration::from_millis(500), "{took:?}");
        drop(exporter);
        assert!(start.elapsed() < Duration::from_millis(700));
    }

    #[test]
    fn a_slow_dns_lookup_holds_neither_the_send_nor_the_drop() {
        let start = Instant::now();
        let exporter = HttpExporter::with_client(
            "http://slow.invalid:4318/v1/traces".into(),
            Some(Duration::from_millis(150)),
            reqwest::Client::builder().dns_resolver(Arc::new(SlowResolver)),
        )
        .unwrap();
        assert!(exporter.send(Vec::new()).is_err());
        let sent = start.elapsed();
        drop(exporter);
        let dropped = start.elapsed();
        assert!(sent < Duration::from_millis(600), "{sent:?}");
        assert!(dropped < Duration::from_millis(800), "{dropped:?}");
    }
}
