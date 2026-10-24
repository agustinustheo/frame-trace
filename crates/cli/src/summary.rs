//! Terminal tables for `frame-trace summary`.

use std::collections::BTreeMap;

use comfy_table::{CellAlignment, Table};
use frame_trace_decode::{AnnotatedBlock, AnnotatedOp};
use frame_trace_model::Stats;

/// Renders the phase table, the top storage items, and the duplicate reads of each phase.
pub fn render(block: &AnnotatedBlock, top: usize) -> String {
    let mut out = if let Some(call) = &block.call {
        format!(
            "call {} at #{} ({})\n\n",
            call.method,
            block.block_number.unwrap_or(0),
            block.block_hash
        )
    } else {
        format!(
            "block {} (parent {})\n\n",
            block.block_hash, block.parent_hash
        )
    };
    out += &format!("Storage per phase\n{}\n\n", phases(block));
    out += &format!(
        "Top {top} storage items by reads + writes\n{}\n\n",
        items(block, top)
    );
    out += &format!(
        "Duplicate reads per phase (top {top} each)\n{}\n",
        duplicates(block, top)
    );
    out
}

fn phases(block: &AnnotatedBlock) -> Table {
    let mut table = table(
        1,
        &[
            "phase",
            "ops",
            "reads",
            "writes",
            "keys",
            "bytes read",
            "bytes written",
            "dup reads",
        ],
    );
    for phase in &block.phases {
        table.add_row(stats_row(phase.label.clone(), &phase.stats));
        // Runs inside the extrinsic (between its checks and its own work); not in its row.
        if let Some(nested) = &phase.post_inherents {
            table.add_row(stats_row(format!("  └ {}", nested.label), &nested.stats));
        }
    }
    table.add_row(stats_row("total".into(), &block.totals));
    table
}

fn stats_row(label: String, s: &Stats) -> Vec<String> {
    let mut row = vec![label];
    row.extend(
        [
            s.ops,
            s.reads,
            s.writes,
            s.distinct_keys,
            s.bytes_read,
            s.bytes_written,
            s.duplicate_reads,
        ]
        .map(|n| n.to_string()),
    );
    row
}

#[derive(Default)]
struct ItemTally {
    reads: usize,
    writes: usize,
    duplicate_reads: usize,
    bytes_read: usize,
    bytes_written: usize,
}

fn items(block: &AnnotatedBlock, top: usize) -> Table {
    let mut tally: BTreeMap<&str, ItemTally> = BTreeMap::new();
    for op in block.all_phases().flat_map(|p| &p.ops) {
        let t = tally.entry(op.item.as_str()).or_default();
        let len = op.value_len.unwrap_or(0);
        match op.op {
            "get" | "exists" | "hash" => {
                t.reads += 1;
                t.bytes_read += len;
                t.duplicate_reads += usize::from(op.duplicate_read);
            }
            "storage_root" | "index_transaction" | "renew_transaction_index" => {}
            _ => {
                t.writes += 1;
                t.bytes_written += len;
            }
        }
    }
    let mut rows: Vec<_> = tally
        .into_iter()
        .filter(|(_, t)| t.reads + t.writes > 0)
        .collect();
    rows.sort_by(|a, b| {
        (b.1.reads + b.1.writes)
            .cmp(&(a.1.reads + a.1.writes))
            .then(a.0.cmp(b.0))
    });

    let mut table = table(
        1,
        &[
            "item",
            "reads",
            "writes",
            "dup reads",
            "bytes read",
            "bytes written",
        ],
    );
    for (item, t) in rows.into_iter().take(top) {
        table.add_row(vec![
            item.to_owned(),
            t.reads.to_string(),
            t.writes.to_string(),
            t.duplicate_reads.to_string(),
            t.bytes_read.to_string(),
            t.bytes_written.to_string(),
        ]);
    }
    table
}

fn duplicates(block: &AnnotatedBlock, top: usize) -> Table {
    let mut table = table(2, &["phase", "key", "dup reads"]);
    for phase in block.all_phases() {
        // Group by key identity (child trie, raw key), whatever read method was used: names
        // can be shared by different keys (`<unknown>`, undecoded map keys).
        let mut counts: BTreeMap<(Option<&str>, &str), (&AnnotatedOp, usize)> = BTreeMap::new();
        for op in phase.ops.iter().filter(|op| op.duplicate_read) {
            counts
                .entry((op.child_trie.as_deref(), op.key.as_str()))
                .or_insert((op, 0))
                .1 += 1;
        }
        let mut rows: Vec<_> = counts.into_values().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.seq.cmp(&b.0.seq)));
        for (op, n) in rows.into_iter().take(top) {
            table.add_row(vec![phase.label.clone(), key_name(op), n.to_string()]);
        }
    }
    table
}

/// The op's key name, with the start of the raw key when the name alone does not identify it.
fn key_name(op: &AnnotatedOp) -> String {
    if op.key_complete {
        return op.name.clone();
    }
    match op.key.get(..34) {
        Some(head) if op.key.len() > 34 => format!("{} [{head}…]", op.name),
        _ => format!("{} [{}]", op.name, op.key),
    }
}

/// A table with the given header; columns after the first `text_columns` hold numbers and are
/// right-aligned.
fn table(text_columns: usize, header: &[&str]) -> Table {
    let mut table = Table::new();
    table
        .load_style(comfy_table::presets::UTF8_FULL_CONDENSED)
        .set_header(header.to_vec());
    for column in table.column_iter_mut().skip(text_columns) {
        column.set_cell_alignment(CellAlignment::Right);
    }
    table
}
