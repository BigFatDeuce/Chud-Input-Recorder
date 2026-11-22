//! Open documents: the backend owns each timeline and its undo history; the
//! interface keeps a mirror updated from the changes returned here.

use std::collections::HashMap;
use std::path::PathBuf;

use chud_core::document::{ContextSample, Environment, MacroDoc, MacroMeta, Marker};
use chud_core::event::{Event, EventId, EventKind, Micros};
use chud_core::flow::Flow;
use chud_core::history::{History, HistoryInfo};
use chud_core::ops::{self, Clip};
use chud_core::playback::PlaybackSettings;
use chud_core::timeline::{Change, Timeline};
use chud_core::transform::{self, CleanupOptions, Scope};
use serde::{Deserialize, Serialize};

pub struct OpenDoc {
    pub doc: MacroDoc,
    pub history: History,
    pub path: PathBuf,
    /// Set when meta/settings/markers/flow changed (not part of undo).
    pub extra_dirty: bool,
}

impl OpenDoc {
    pub fn new(doc: MacroDoc, path: PathBuf) -> Self {
        Self { doc, history: History::default(), path, extra_dirty: false }
    }
    pub fn dirty(&self) -> bool {
        self.history.is_dirty() || self.extra_dirty
    }
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
        self.extra_dirty = false;
    }
}

#[derive(Default)]
pub struct Docs {
    pub open: HashMap<String, OpenDoc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocPayload {
    pub meta: MacroMeta,
    pub settings: PlaybackSettings,
    pub env: Environment,
    pub events: Vec<Event>,
    pub markers: Vec<Marker>,
    pub contexts: Vec<ContextSample>,
    pub flow: Option<Flow>,
    /// Images searched on screen by the flow.
    pub images: Vec<crate::screen::DocImage>,
    pub has_original: bool,
    pub history: HistoryInfo,
    pub dirty: bool,
    pub path: String,
}

impl DocPayload {
    pub fn of(o: &OpenDoc) -> Self {
        let d = &o.doc;
        Self {
            meta: d.meta.clone(),
            settings: d.settings.clone(),
            env: d.env.clone(),
            events: d.timeline.events().to_vec(),
            markers: d.markers.clone(),
            contexts: d.contexts.clone(),
            flow: d.flow.clone(),
            images: d.images.iter().map(|(id, png)| crate::screen::doc_image(id, png)).collect(),
            has_original: d.original.is_some(),
            history: o.history.info(),
            dirty: o.dirty(),
            path: o.path.to_string_lossy().to_string(),
        }
    }
}

/// An event edit from the inspector.
#[derive(Debug, Clone, Deserialize)]
pub struct EventEdit {
    pub id: EventId,
    pub t: Micros,
    pub kind: EventKind,
}

fn default_true() -> bool {
    true
}

/// Every editing operation the interface can request.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    Delete {
        ids: Vec<EventId>,
        #[serde(default = "default_true")]
        partners: bool,
    },
    Insert {
        items: Vec<(Micros, EventKind)>,
    },
    Update {
        edits: Vec<EventEdit>,
    },
    Shift {
        ids: Vec<EventId>,
        dt: Micros,
    },
    Translate {
        ids: Vec<EventId>,
        dx: i32,
        dy: i32,
    },
    Scale {
        ids: Vec<EventId>,
        anchor: Micros,
        factor: f64,
    },
    Quantize {
        ids: Vec<EventId>,
        grid: Micros,
        strength: f64,
    },
    SetGaps {
        ids: Vec<EventId>,
        gap: Micros,
        simultaneous: Micros,
    },
    SetHoldDuration {
        id: EventId,
        duration: Micros,
    },
    Paste {
        items: Vec<(Micros, EventKind)>,
        at: Micros,
    },
    Duplicate {
        ids: Vec<EventId>,
        offset: Option<Micros>,
    },
    InsertTime {
        at: Micros,
        dt: Micros,
    },
    DeleteRange {
        start: Micros,
        end: Micros,
        ripple: bool,
    },
    RemoveDeadTime {
        threshold: Micros,
        keep: Micros,
        respect_holds: bool,
    },
    SimplifyMouse {
        ids: Option<Vec<EventId>>,
        epsilon: f64,
        max_interval: Micros,
    },
    MergeRawMotion {
        ids: Option<Vec<EventId>>,
        interval: Micros,
    },
    MergeWheel {
        window: Micros,
    },
    Cleanup {
        options: CleanupOptions,
    },
    UniformTiming {
        gap: Micros,
        simultaneous: Micros,
    },
    /// Changes the speed of the whole timeline (factor > 1 = faster).
    Speed {
        factor: f64,
    },
    RevertOriginal,
}

impl EditOp {
    pub fn label(&self) -> &'static str {
        match self {
            EditOp::Delete { .. } => "delete",
            EditOp::Insert { .. } => "insert",
            EditOp::Update { .. } => "edit",
            EditOp::Shift { .. } => "move",
            EditOp::Translate { .. } => "move_position",
            EditOp::Scale { .. } => "stretch",
            EditOp::Quantize { .. } => "quantize",
            EditOp::SetGaps { .. } => "set_gaps",
            EditOp::SetHoldDuration { .. } => "hold_duration",
            EditOp::Paste { .. } => "paste",
            EditOp::Duplicate { .. } => "duplicate",
            EditOp::InsertTime { .. } => "insert_time",
            EditOp::DeleteRange { .. } => "delete_range",
            EditOp::RemoveDeadTime { .. } => "remove_dead_time",
            EditOp::SimplifyMouse { .. } => "simplify_mouse",
            EditOp::MergeRawMotion { .. } => "merge_raw",
            EditOp::MergeWheel { .. } => "merge_wheel",
            EditOp::Cleanup { .. } => "cleanup",
            EditOp::UniformTiming { .. } => "uniform_timing",
            EditOp::Speed { .. } => "speed",
            EditOp::RevertOriginal => "revert_original",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EditResult {
    pub removed: Vec<EventId>,
    pub added: Vec<Event>,
    pub history: HistoryInfo,
    pub dirty: bool,
    /// Ids created by insert/paste/duplicate (to select them).
    pub created: Vec<EventId>,
    /// Operation specific details (cleanup report…).
    pub report: Option<serde_json::Value>,
}

pub fn result_of(o: &OpenDoc, change: &Change, created: Vec<EventId>, report: Option<serde_json::Value>) -> EditResult {
    EditResult {
        removed: change.removed.iter().map(|e| e.id).collect(),
        added: change.added.clone(),
        history: o.history.info(),
        dirty: o.dirty(),
        created,
        report,
    }
}

fn scope(ids: &Option<Vec<EventId>>) -> Scope {
    Scope { ids: ids.clone() }
}

/// Computes the change for an operation (does not apply it).
pub fn compute(tl: &Timeline, original: Option<&Vec<(Micros, EventKind)>>, op: &EditOp) -> Result<(Change, Vec<EventId>, Option<serde_json::Value>), String> {
    let none = |c: Change| Ok((c, Vec::new(), None));
    match op {
        EditOp::Delete { ids, partners } => none(ops::delete(tl, ids, *partners)),
        EditOp::Insert { items } => {
            let (c, ids) = ops::insert(tl, items);
            Ok((c, ids, None))
        }
        EditOp::Update { edits } => {
            let e: Vec<_> = edits.iter().map(|e| (e.id, e.t.max(0), e.kind)).collect();
            none(ops::update(tl, &e))
        }
        EditOp::Shift { ids, dt } => none(ops::shift(tl, ids, *dt)),
        EditOp::Translate { ids, dx, dy } => none(ops::translate(tl, ids, *dx, *dy)),
        EditOp::Scale { ids, anchor, factor } => {
            if !(factor.is_finite() && *factor > 0.0) {
                return Err("invalid factor".into());
            }
            none(ops::scale(tl, ids, *anchor, *factor))
        }
        EditOp::Quantize { ids, grid, strength } => none(ops::quantize(tl, ids, (*grid).max(1), strength.clamp(0.0, 1.0))),
        EditOp::SetGaps { ids, gap, simultaneous } => none(ops::set_gaps(tl, ids, (*gap).max(0), (*simultaneous).max(0))),
        EditOp::SetHoldDuration { id, duration } => none(ops::set_hold_duration(tl, *id, (*duration).max(0))),
        EditOp::Paste { items, at } => {
            let (c, ids) = ops::paste(tl, &Clip { items: items.clone() }, (*at).max(0));
            Ok((c, ids, None))
        }
        EditOp::Duplicate { ids, offset } => {
            let (c, ids) = ops::duplicate(tl, ids, *offset);
            Ok((c, ids, None))
        }
        EditOp::InsertTime { at, dt } => none(ops::insert_time(tl, (*at).max(0), *dt)),
        EditOp::DeleteRange { start, end, ripple } => none(ops::delete_range(tl, *start.min(end), *start.max(end), *ripple)),
        EditOp::RemoveDeadTime { threshold, keep, respect_holds } => {
            none(transform::remove_dead_time(tl, (*threshold).max(1), (*keep).max(0), *respect_holds))
        }
        EditOp::SimplifyMouse { ids, epsilon, max_interval } => {
            none(transform::simplify_mouse(tl, &scope(ids), epsilon.max(0.0), (*max_interval).max(0)))
        }
        EditOp::MergeRawMotion { ids, interval } => none(transform::merge_raw_motion(tl, &scope(ids), (*interval).max(1))),
        EditOp::MergeWheel { window } => none(transform::merge_wheel(tl, (*window).max(1))),
        EditOp::Cleanup { options } => {
            let (c, report) = transform::cleanup(tl, *options);
            Ok((c, Vec::new(), serde_json::to_value(report).ok()))
        }
        EditOp::UniformTiming { gap, simultaneous } => none(transform::uniform_timing(tl, (*gap).max(0), (*simultaneous).max(0))),
        EditOp::Speed { factor } => {
            if !(factor.is_finite() && *factor > 0.0) {
                return Err("invalid speed".into());
            }
            let ids: Vec<EventId> = tl.events().iter().map(|e| e.id).collect();
            none(ops::scale(tl, &ids, 0, 1.0 / factor))
        }
        EditOp::RevertOriginal => {
            let orig = original.ok_or("this macro has no original take")?;
            let next = tl.next_id();
            let added: Vec<Event> = orig
                .iter()
                .enumerate()
                .map(|(i, (t, kind))| Event { id: next + i as EventId, t: *t, kind: *kind })
                .collect();
            let mut added = added;
            added.sort_by(Event::order);
            none(Change { removed: tl.events().to_vec(), added })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chud_core::event::Key;

    fn od() -> OpenDoc {
        let k = Key::new(0x1E, 0x41);
        let doc = MacroDoc::from_recording(
            "t",
            vec![
                (0, EventKind::KeyDown { key: k, repeat: false }),
                (5_000, EventKind::KeyUp { key: k }),
                (2_000_000, EventKind::MouseMove { x: 1, y: 1 }),
            ],
            Default::default(),
        );
        OpenDoc::new(doc, PathBuf::new())
    }

    fn apply(o: &mut OpenDoc, op: EditOp) -> EditResult {
        let (c, created, report) = compute(&o.doc.timeline, o.doc.original.as_ref(), &op).unwrap();
        o.history.commit(&mut o.doc.timeline, op.label(), c.clone()).unwrap();
        result_of(o, &c, created, report)
    }

    #[test]
    fn ops_roundtrip_through_json_and_history() {
        let mut o = od();
        let op: EditOp = serde_json::from_str(r#"{"op":"speed","factor":2.0}"#).unwrap();
        let r = apply(&mut o, op);
        assert!(r.dirty && r.history.can_undo);
        assert_eq!(o.doc.timeline.duration(), 1_000_000);
        let op: EditOp = serde_json::from_str(r#"{"op":"delete","ids":[0]}"#).unwrap();
        let r = apply(&mut o, op);
        assert_eq!(r.removed.len(), 2, "press and its release");
        let r = apply(&mut o, EditOp::RevertOriginal);
        assert_eq!(r.added.len(), 3);
        assert_eq!(o.doc.timeline.duration(), 2_000_000);
        o.doc.timeline.check_invariants().unwrap();
        let ins: EditOp = serde_json::from_str(
            r#"{"op":"insert","items":[[10,{"type":"wheel","delta":120,"horizontal":false,"x":0,"y":0}]]}"#,
        )
        .unwrap();
        let r = apply(&mut o, ins);
        assert_eq!(r.created.len(), 1);
        o.history.undo(&mut o.doc.timeline).unwrap();
        assert_eq!(o.doc.timeline.len(), 3);
    }
}
