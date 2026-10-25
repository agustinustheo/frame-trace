use parking_lot::Mutex;
use serde::Serialize;
use std::{collections::BTreeMap, fmt, sync::Arc, time::Instant};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    Metadata, Subscriber,
};

#[derive(Default)]
struct Fields(BTreeMap<String, String>);
impl Visit for Fields {
    fn record_debug(&mut self, f: &Field, v: &dyn fmt::Debug) {
        self.0.insert(f.name().into(), format!("{v:?}"));
    }
    fn record_str(&mut self, f: &Field, v: &str) {
        self.0.insert(f.name().into(), v.into());
    }
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.0.insert(f.name().into(), v.to_string());
    }
    fn record_i64(&mut self, f: &Field, v: i64) {
        self.0.insert(f.name().into(), v.to_string());
    }
    fn record_bool(&mut self, f: &Field, v: bool) {
        self.0.insert(f.name().into(), v.to_string());
    }
}

#[derive(Clone, Serialize)]
pub struct Span {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub target: String,
    pub wasm: bool,
    pub fields: BTreeMap<String, String>,
    pub start_ns: u64,
    pub end_ns: Option<u64>,
    pub duration_ns: u64,
    pub enters: u32,
    pub proof_start_bytes: usize,
    pub proof_end_bytes: usize,
    #[serde(skip)]
    entered: Option<Instant>,
}

#[derive(Clone, Serialize)]
pub struct Event {
    pub seq: usize,
    pub at_ns: u64,
    pub parent_id: Option<u64>,
    pub target: String,
    pub fields: BTreeMap<String, String>,
}

pub struct Data {
    pub start: Instant,
    pub spans: Vec<Span>,
    pub events: Vec<Event>,
    stack: Vec<u64>,
    metadata: Vec<&'static Metadata<'static>>,
    pub recorder: Option<sp_trie::recorder::Recorder<sp_core::Blake2Hasher>>,
}

#[derive(Clone)]
pub struct Capture(pub Arc<Mutex<Data>>);
impl Capture {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Data {
            start: Instant::now(),
            spans: Vec::new(),
            events: Vec::new(),
            stack: Vec::new(),
            metadata: Vec::new(),
            recorder: None,
        })))
    }
}

impl Subscriber for Capture {
    fn enabled(&self, m: &Metadata<'_>) -> bool {
        m.target() == "state"
            || m.target() == "wasm_tracing"
            || m.target() == "e2_backend"
            || m.target().starts_with("sp_io::storage")
            || m.target().starts_with("sp_io::default_child_storage")
            || m.target()
                .starts_with("cumulus_primitives_proof_size_hostfunction")
            || m.target() == "e2"
            || *m.level() <= tracing::Level::WARN
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        let mut d = self.0.lock();
        let id = d.spans.len() as u64 + 1;
        // Wasm transport uses explicit None for contextual parents. Recover
        // synchronous nesting from enter/exit callbacks, including in that case.
        let parent_id = attrs
            .parent()
            .map(Id::into_u64)
            .or_else(|| d.stack.last().copied());
        let wasm = attrs.metadata().target() == "wasm_tracing";
        let name = if wasm {
            fields.0.get("name").cloned().unwrap_or_default()
        } else {
            attrs.metadata().name().into()
        };
        let target = if wasm {
            fields.0.get("target").cloned().unwrap_or_default()
        } else {
            attrs.metadata().target().into()
        };
        let start_ns = d.start.elapsed().as_nanos() as u64;
        d.spans.push(Span {
            id,
            parent_id,
            name,
            target,
            wasm,
            fields: fields.0,
            start_ns,
            end_ns: None,
            duration_ns: 0,
            enters: 0,
            entered: None,
            proof_start_bytes: 0,
            proof_end_bytes: 0,
        });
        d.metadata.push(attrs.metadata());
        Id::from_u64(id)
    }
    fn record(&self, id: &Id, values: &Record<'_>) {
        let mut fields = Fields::default();
        values.record(&mut fields);
        self.0.lock().spans[id.into_u64() as usize - 1]
            .fields
            .extend(fields.0);
    }
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let mut d = self.0.lock();
        let parent_id = event
            .parent()
            .map(Id::into_u64)
            .or_else(|| d.stack.last().copied());
        let at_ns = d.start.elapsed().as_nanos() as u64;
        let seq = d.events.len();
        d.events.push(Event {
            seq,
            at_ns,
            parent_id,
            target: event.metadata().target().into(),
            fields: fields.0,
        });
    }
    fn enter(&self, id: &Id) {
        let mut d = self.0.lock();
        d.stack.push(id.into_u64());
        let proof_size = d
            .recorder
            .as_ref()
            .map(|r| r.estimate_encoded_size())
            .unwrap_or(0);
        let s = &mut d.spans[id.into_u64() as usize - 1];
        s.enters += 1;
        s.entered = Some(Instant::now());
        s.proof_start_bytes = proof_size;
    }
    fn exit(&self, id: &Id) {
        let mut d = self.0.lock();
        if let Some(pos) = d.stack.iter().rposition(|v| *v == id.into_u64()) {
            d.stack.remove(pos);
        }
        let now = d.start.elapsed().as_nanos() as u64;
        let proof_size = d
            .recorder
            .as_ref()
            .map(|r| r.estimate_encoded_size())
            .unwrap_or(0);
        let s = &mut d.spans[id.into_u64() as usize - 1];
        if let Some(start) = s.entered.take() {
            s.duration_ns += start.elapsed().as_nanos() as u64;
        }
        s.end_ns = Some(now);
        s.proof_end_bytes = proof_size;
    }
    fn clone_span(&self, id: &Id) -> Id {
        id.clone()
    }
    fn try_close(&self, _: Id) -> bool {
        false
    }
    fn current_span(&self) -> tracing_core::span::Current {
        let d = self.0.lock();
        match d.stack.last() {
            Some(id) => {
                tracing_core::span::Current::new(Id::from_u64(*id), d.metadata[*id as usize - 1])
            }
            None => tracing_core::span::Current::none(),
        }
    }
}
