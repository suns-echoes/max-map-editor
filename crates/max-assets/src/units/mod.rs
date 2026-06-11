//! Base-unit-data records referenced by `D_*` entries in the RES archive.
//!
//! Each record holds 8 `u8` fields pointing into a unit's sprite strips,
//! plus 8 path-step offsets (one per rotation). These drive the renderer's
//! choice of sprite index given a unit's current heading and state.

mod base_data;
pub use base_data::{BaseUnitData, parse_base_unit_data};
