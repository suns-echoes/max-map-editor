//! The Resize Map dialog: size preset + W/H fields + the 3x3 anchor radio
//! grid, with the live offset note, running the same `resize` command line.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct ResizeIds {
	pub(super) resize: WidgetId,
	pub(super) cancel: WidgetId,
	pub(super) preset: WidgetId,
	pub(super) width: WidgetId,
	pub(super) height: WidgetId,
	pub(super) note: WidgetId,
}

impl Overlay {
	/// Opens the Resize Map form: a size preset, W/H fields, a 3x3 anchor radio
	/// grid (where the old map sits in the new bounds), and a live offset note.
	pub fn open_resize(&mut self, old_w: u16, old_h: u16) {
		let presets: Vec<String> = SIZE_PRESETS
			.iter()
			.map(|(n, _, _)| (*n).to_string())
			.chain(std::iter::once("Custom".to_string()))
			.collect();
		let preset = Select::new(presets);
		let width = TextInput::with_text(old_w.to_string());
		let height = TextInput::with_text(old_h.to_string());
		let note = Label::new("").small().with_id();
		let resize = Button::new("Resize").primary();
		let cancel = Button::new("Abort").secondary();
		let ids = ResizeIds {
			resize: resize.id(),
			cancel: cancel.id(),
			preset: preset.id(),
			width: width.id(),
			height: height.id(),
			note: note.id(),
		};
		// Size row: [Width] x [Height].
		let size_row = Linear::row()
			.spacing(8.0)
			.cross_align(CrossAlign::Center)
			.child(Label::new("Size").small(), Length::Fixed(78.0))
			.child(width, Length::Fixed(64.0))
			.push(Label::new("x"))
			.child(height, Length::Fixed(64.0));
		// 3x3 anchor grid of radios (centre selected); the host manages the group.
		let mut grid = Linear::column().spacing(6.0);
		let mut anchor_ids = Vec::with_capacity(9);
		for row in 0..3u8 {
			let mut r = Linear::row().spacing(6.0);
			for col in 0..3u8 {
				let idx = (row * 3 + col) as usize;
				let rb = Radio::new("").with_selected(idx == 4);
				anchor_ids.push(rb.id());
				r = r.push(rb);
			}
			grid = grid.push(r);
		}
		let content = column()
			.push(width_strut(300.0))
			.push(field_row("Preset", preset))
			.push(size_row)
			.push(Label::new("Anchor").small())
			.push(grid);
		let content = status_slot(content, note, 300.0, 2).push(buttons(cancel, resize));
		let win = dialog("Resize Map", content);
		self.win_id = Some(win.id());
		self.ui = Ui::new(modal(win));
		self.ui.focus_first();
		self.dialog = Dialog::Resize(ids);
		self.anchor_ids = anchor_ids;
		self.resize_old = (old_w, old_h);
		self.events.clear();
		self.visible = true;
	}

	/// The selected anchor `(col, row)` in `0..3` (row-major radios; centre = 1,1).
	fn anchor(&self) -> (u8, u8) {
		let idx =
			self.anchor_ids.iter().position(|id| self.ui.get::<Radio>(*id).is_some_and(Radio::selected)).unwrap_or(4);
		((idx % 3) as u8, (idx / 3) as u8)
	}

	/// The W/H fields parsed and range-checked (1..=1024), or the reason a
	/// field fails - shown live in the dialog's note slot.
	pub(super) fn resize_dims(&self, ids: &ResizeIds) -> Result<(u16, u16), String> {
		let w = parse_dim(&self.text(ids.width)).map_err(|e| format!("width {e}"))?;
		let h = parse_dim(&self.text(ids.height)).map_err(|e| format!("height {e}"))?;
		Ok((w, h))
	}

	/// The old map's offset inside the new bounds from the 3x3 anchor (col/row
	/// 0 = top/left edge, 1 = centred, 2 = bottom/right edge).
	pub(super) fn resize_offset(&self, new_w: u16, new_h: u16) -> (i32, i32) {
		let (col, row) = self.anchor();
		let (ow, oh) = self.resize_old;
		(col as i32 * (new_w as i32 - ow as i32) / 2, row as i32 * (new_h as i32 - oh as i32) / 2)
	}

	/// The validated `resize W H OFFX OFFY` command line, or `None` if invalid
	/// (the note slot is already saying why).
	pub(super) fn resize_command(&self, ids: &ResizeIds) -> Option<String> {
		let (w, h) = self.resize_dims(ids).ok()?;
		let (ox, oy) = self.resize_offset(w, h);
		Some(format!("resize {w} {h} {ox} {oy}"))
	}

	/// The live offset note (verb + what fills/crops + the derived offset) - or,
	/// when a field is bad, the reason the Resize key will refuse.
	pub(super) fn resize_note(&self, ids: &ResizeIds) -> String {
		let (w, h) = match self.resize_dims(ids) {
			Ok(dims) => dims,
			Err(why) => return why,
		};
		let (ow, oh) = self.resize_old;
		let (ox, oy) = self.resize_offset(w, h);
		let verb = if w >= ow && h >= oh {
			"Enlarge - fills with water"
		} else if w <= ow && h <= oh {
			"Shrink - crops to the anchor"
		} else {
			"Resize - fills and crops"
		};
		format!("{verb}   from {ow} x {oh}, at {ox}, {oy}")
	}

	/// One frame of the dialog's control dispatch (the render match arm).
	pub(super) fn dispatch_resize(&mut self, ids: ResizeIds) -> Outcome {
		let mut outcome = Outcome::Idle;
		// A preset selection fills the W/H fields (Custom leaves them).
		if self.ui.fired(ids.preset) {
			let i = self.ui.get::<Select>(ids.preset).map(Select::selected).unwrap_or(0);
			if let Some((_, w, h)) = SIZE_PRESETS.get(i) {
				self.set_text(ids.width, &w.to_string());
				self.set_text(ids.height, &h.to_string());
			}
		}
		// The anchor radios are one group: clear the others when one fires.
		let group = self.anchor_ids.clone();
		self.radio_group(&group);
		// Keep the offset note live as W/H / anchor change.
		let note = self.resize_note(&ids);
		self.set_label(ids.note, &note);
		if self.ui.fired(ids.cancel) {
			self.hide();
		} else if self.ui.fired(ids.resize) {
			if let Some(cmd) = self.resize_command(&ids) {
				outcome = Outcome::ResizeMap(cmd);
				self.hide();
			}
		}
		outcome
	}
}
