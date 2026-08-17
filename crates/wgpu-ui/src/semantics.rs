//! Structural queries and a diffable outline over a widget tree — the test
//! story. A logic test asks "is there exactly one Button labeled Unlock in
//! this dialog" instead of comparing pixels: faster, diffable, nothing to
//! re-bless. A script driver addresses widgets by the label a user reads
//! instead of threading ids through host code. Both read
//! [`Widget::semantics`]; widgets that don't override it appear under their
//! bare type name, unlabeled.
//!
//! The outline format — `Kind "label" (x,y wxh) [flags]`, children indented
//! two spaces — is a stable test artifact: extend it, don't reshuffle it.
//! Every failing `expect_*` panics with the outline of the searched subtree,
//! so a failure shows what the tree actually was.

use std::fmt::Write;

use crate::interact::WidgetId;
use crate::widget::Widget;

/// What [`outline`] prints per widget beyond kind + label.
#[derive(Debug, Clone, Copy)]
pub struct Outline {
    /// Append `(x,y wxh)` rects (off by default: geometry churn would make
    /// purely structural expectations brittle).
    pub geometry: bool,
    /// Append `[flags]`: `focusable`, plus `focused`/`hovered` against the
    /// ids below.
    pub state: bool,
    /// The hovered/focused ids the state flags compare against — they live on
    /// the [`Ui`](crate::ui::Ui), not in the tree, so the plain constructors
    /// leave them `NONE` and [`Ui::outline`](crate::ui::Ui::outline) fills
    /// them in.
    pub hovered: WidgetId,
    pub focused: WidgetId,
}

impl Outline {
    /// Structure only: kinds and labels.
    #[must_use]
    pub fn structure() -> Outline {
        Outline {
            geometry: false,
            state: false,
            hovered: WidgetId::NONE,
            focused: WidgetId::NONE,
        }
    }

    #[must_use]
    pub fn with_geometry(mut self) -> Outline {
        self.geometry = true;
        self
    }

    #[must_use]
    pub fn with_state(mut self) -> Outline {
        self.state = true;
        self
    }
}

/// An indented, diffable dump of the subtree — one line per widget:
/// `Kind "label" (x,y wxh) [flags]`, children indented two spaces.
#[must_use]
pub fn outline(from: &dyn Widget, opts: &Outline) -> String {
    let mut out = String::new();
    write_outline(from, opts, 0, &mut out);
    out
}

fn write_outline(w: &dyn Widget, opts: &Outline, depth: usize, out: &mut String) {
    let sem = w.semantics();
    out.push_str(&"  ".repeat(depth));
    out.push_str(sem.kind);
    if let Some(label) = sem.label {
        let _ = write!(out, " \"{label}\"");
    }
    if opts.geometry {
        let r = w.rect();
        let _ = write!(out, " ({},{} {}x{})", r.x, r.y, r.w, r.h);
    }
    if opts.state {
        let mut flags: Vec<&str> = Vec::new();
        if w.accepts_focus() {
            flags.push("focusable");
        }
        let id = w.id();
        if id != WidgetId::NONE {
            if id == opts.focused {
                flags.push("focused");
            }
            if id == opts.hovered {
                flags.push("hovered");
            }
        }
        if !flags.is_empty() {
            let _ = write!(out, " [{}]", flags.join(", "));
        }
    }
    out.push('\n');
    for i in 0..w.child_count() {
        if let Some(c) = w.child(i) {
            write_outline(c, opts, depth + 1, out);
        }
    }
}

/// Every widget under `from` (inclusive, pre-order) matching `pred`. Borrowed,
/// not id'd: anonymous widgets (containers) match too, and a hit's id, rect,
/// and typed state are all one call away.
pub fn find_all(from: &dyn Widget, mut pred: impl FnMut(&dyn Widget) -> bool) -> Vec<&dyn Widget> {
    let mut out = Vec::new();
    collect(from, &mut pred, &mut out);
    out
}

fn collect<'a>(
    w: &'a dyn Widget,
    pred: &mut dyn FnMut(&dyn Widget) -> bool,
    out: &mut Vec<&'a dyn Widget>,
) {
    if pred(w) {
        out.push(w);
    }
    for i in 0..w.child_count() {
        if let Some(c) = w.child(i) {
            collect(c, pred, out);
        }
    }
}

/// Widgets under `from` of this semantic kind (normally the type name:
/// `by_kind(root, "Button")`).
#[must_use]
pub fn by_kind<'a>(from: &'a dyn Widget, kind: &str) -> Vec<&'a dyn Widget> {
    find_all(from, |w| w.semantics().kind == kind)
}

/// Widgets under `from` whose label equals `label`.
#[must_use]
pub fn by_label<'a>(from: &'a dyn Widget, label: &str) -> Vec<&'a dyn Widget> {
    find_all(from, |w| w.semantics().label == Some(label))
}

/// Exactly one widget under `from` matching `pred`, described as `what` in
/// failures.
///
/// # Panics
/// With the subtree outline when zero or several match.
pub fn expect_one<'a>(
    from: &'a dyn Widget,
    what: &str,
    pred: impl FnMut(&dyn Widget) -> bool,
) -> &'a dyn Widget {
    let found = find_all(from, pred);
    match found.as_slice() {
        [one] => *one,
        [] => panic!(
            "no widget matched: {what}\n--- tree ---\n{}",
            outline(from, &Outline::structure().with_state())
        ),
        many => panic!(
            "{} widgets matched: {what}\n--- tree ---\n{}",
            many.len(),
            outline(from, &Outline::structure().with_state())
        ),
    }
}

/// Exactly one widget under `from` with this label — the script-driver
/// workhorse: `expect_labeled(ui.root(), "OK").id()`.
///
/// # Panics
/// With the subtree outline when zero or several match.
pub fn expect_labeled<'a>(from: &'a dyn Widget, label: &str) -> &'a dyn Widget {
    expect_one(from, &format!("label \"{label}\""), |w| {
        w.semantics().label == Some(label)
    })
}
