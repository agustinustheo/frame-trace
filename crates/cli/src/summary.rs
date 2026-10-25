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
    out += &crate::report::render(&crate::report::aggregate(std::slice::from_ref(block)), top);
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
            "proof B",
            "storage µs",
            "wall µs",
        ],
    );
    for phase in &block.phases {
        table.add_row(phase_row(phase, phase.label.clone()));
        // Runs inside the extrinsic (between its checks and its own work); not in its row.
        if let Some(nested) = &phase.post_inherents {
            table.add_row(phase_row(nested, format!("  └ {}", nested.label)));
        }
    }
    let mut total = stats_row("total".into(), &block.totals);
    let tally = crate::report::Tally::of(block.all_phases().flat_map(|p| &p.ops));
    total.extend([
        crate::report::proof_label(&tally),
        format!("{:.1}", tally.storage_ns as f64 / 1000.0),
        block.timing.map_or_else(
            || "?".into(),
            |t| format!("{:.1}", t.duration_ns as f64 / 1000.0),
        ),
    ]);
    table.add_row(total);
    table
}

fn phase_row(phase: &frame_trace_decode::AnnotatedPhase, label: String) -> Vec<String> {
    let mut row = stats_row(label, &phase.stats);
    let t = crate::report::Tally::of(&phase.ops);
    row.extend([
        crate::report::proof_label(&t),
        format!("{:.1}", t.storage_ns as f64 / 1000.0),
        phase.wall_time.as_ref().map_or_else(
            || "?".into(),
            |t| format!("{:.1}", t.duration_ns as f64 / 1000.0),
        ),
    ]);
    row
}
fn stats_row(label: String, s: &Stats) -> Vec<String> {
    let mut row = vec![label];
    row.extend([s.ops, s.reads, s.writes].map(|n| n.to_string()));
    row
}

fn duplicates(block: &AnnotatedBlock, top: usize) -> Table {
    let mut table = table(
        2,
        &[
            "phase",
            "key",
            "dup reads",
            "overlay",
            "warm",
            "cold",
            "unknown",
        ],
    );
    for phase in block.all_phases() {
        // Group by key identity (child trie, raw key), whatever read method was used: names
        // can be shared by different keys (`<unknown>`, undecoded map keys).
        type DuplicateRows<'a> =
            BTreeMap<(Option<&'a str>, &'a str), (&'a AnnotatedOp, [usize; 5])>;
        let mut counts: DuplicateRows<'_> = BTreeMap::new();
        for op in phase.ops.iter().filter(|op| op.duplicate_read) {
            let row = counts
                .entry((op.child_trie.as_deref(), op.key.as_str()))
                .or_insert((op, [0; 5]));
            row.1[0] += 1;
            row.1[crate::report::origin_index(op.origin.as_deref()) + 1] += 1;
        }
        let mut rows: Vec<_> = counts.into_values().collect();
        rows.sort_by(|a, b| b.1[0].cmp(&a.1[0]).then(a.0.seq.cmp(&b.0.seq)));
        let omitted = rows.len().saturating_sub(top);
        for (op, n) in rows.into_iter().take(top) {
            let mut row = vec![phase.label.clone(), key_name(op)];
            row.extend(n.map(|n| n.to_string()));
            table.add_row(row);
        }
        if omitted > 0 {
            table.add_row(vec![
                phase.label.clone(),
                format!("… {omitted} more"),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
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
pub(crate) fn table(text_columns: usize, header: &[&str]) -> Table {
    let mut table = Table::new();
    table
        .load_style(comfy_table::presets::UTF8_FULL_CONDENSED)
        .set_header(header.to_vec());
    for column in table.column_iter_mut().skip(text_columns) {
        column.set_cell_alignment(CellAlignment::Right);
    }
    table
}
