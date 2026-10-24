//! Knowing whether every span reached the backend.
//!
//! The batch span processor exports on its own thread and only logs export errors, and
//! `force_flush` does not report errors from earlier batches. [`Tracked`] wraps the exporter and
//! counts what each batch did, so [`crate::export_block`] can fail when spans were lost.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    trace::{SpanData, SpanExporter},
};

/// What happened to the spans handed to a [`Tracked`] exporter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delivery {
    /// Spans in batches the inner exporter accepted.
    pub exported: usize,
    /// Spans in batches the inner exporter rejected.
    pub failed: usize,
    /// The first export error.
    pub first_error: Option<String>,
}

/// A span exporter that records the outcome of every batch in a shared [`Delivery`].
#[derive(Debug)]
pub struct Tracked<E> {
    inner: E,
    delivery: Arc<Mutex<Delivery>>,
}

impl<E> Tracked<E> {
    /// Wraps `inner`. The handle reads the tally while the exporter is owned by the SDK.
    pub fn new(inner: E) -> (Self, DeliveryHandle) {
        let delivery = Arc::new(Mutex::new(Delivery::default()));
        (
            Self {
                inner,
                delivery: delivery.clone(),
            },
            DeliveryHandle(delivery),
        )
    }
}

/// Reads the [`Delivery`] of a [`Tracked`] exporter.
#[derive(Debug, Clone)]
pub struct DeliveryHandle(Arc<Mutex<Delivery>>);

impl DeliveryHandle {
    /// The tally so far.
    pub fn get(&self) -> Delivery {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<E: SpanExporter> SpanExporter for Tracked<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let spans = batch.len();
        let result = self.inner.export(batch).await;
        let mut delivery = self.delivery.lock().unwrap_or_else(PoisonError::into_inner);
        match &result {
            Ok(()) => delivery.exported += spans,
            Err(err) => {
                delivery.failed += spans;
                delivery.first_error.get_or_insert_with(|| err.to_string());
            }
        }
        result
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}
