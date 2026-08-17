//! M.A.X. saved-game (`.DTA`) decoder.
//!
//! Reads M.A.X. save files — player saves (`SAVE#.DTA`) and the shipped stock
//! missions in the same binary format (`.CAM/.SCE/.TRA/.MPS/.DMO`). Two on-disk
//! formats exist: `V70` (the original DOS game + stock missions) and `V71`
//! (written by M.A.X. Port); both are detected and handled.
//!
//! Decodes the full save body — header + game options, the surface and cargo
//! maps, per-team state, and the five unit lists resolved against the shared
//! object graph. Bytes in, typed values out; no game logic. See `SAVE-FORMAT.md`
//! for the byte-level spec.

pub mod cargo;
pub mod complexes;
pub mod decode;
pub mod encode;
pub mod error;
pub mod export;
pub mod integrity;
pub mod mining;
pub mod orders;
pub mod read;
pub mod serialize;
pub mod settings;
pub mod tail;
pub mod types;
pub mod unit_names;
pub mod unit_types;

pub use cargo::{
	CARGO_AMOUNT_MASK, CARGO_FUEL, CARGO_GOLD, CARGO_RAW, CARGO_SURVEY_ALL, CargoMaterial, cargo_amount, cargo_compose,
	cargo_material, cargo_surveyed,
};
pub use complexes::{CONNECTOR_BITS, check_complexes, connector_neighbor, dead_listed_complexes, repair_complexes};
pub use decode::{read_save, read_save_bytes};
pub use error::EditError;
pub use export::{
	FreshBodyCtx, UnitScalarEdit, add_unit, apply_stat_override, move_unit, patch_unit_scalars, remove_unit,
};
pub use integrity::{IssueKind, TransientIssue, check_transient_state, repair_transient_state, reset_transient_prefix};
pub use mining::{MININGST, derive_mining, initial_allocation, mining_bytes, repair_mining, set_initial_mining};
pub use orders::{
	ORDER_AWAIT, ORDER_BUILD, ORDER_DISABLE, ORDER_NAMES, ORDER_POWER_OFF, ORDER_POWER_ON, ORDER_SENTRY,
	ORDER_STATE_BUILD_IN_PROGRESS, ORDER_STATE_EXECUTING_ORDER, ORDER_STATE_INIT, deploy_state_for, order_id,
	order_name,
};
pub use read::{SaveError, read_save_header, stock_world_hash, world_index_from_hash};
pub use serialize::{serialize_complex, serialize_unit_values, write_save};
pub use settings::{RESEARCH_TOPICS, SaveSettings, TeamStats};
pub use tail::{TEAM_TYPE_COMPUTER, TEAM_TYPE_NONE, referenced_units};
pub use types::{
	AirPath, Complex, CtInfo, MapCell, MapHash, ObjMeta, RawRegions, SaveCategory, SaveExtraSettings, SaveFile,
	SaveFormat, SaveHeader, SaveObject, SaveOptions, TEAM_COUNT, TEAM_LABELS, TeamUnitsTable, UNIT_END, UnitBodyLayout,
	UnitPath, UnitRecord, UnitValues, WORLD_FILE_NAMES, world_file_name,
};
pub use unit_names::{PLAYER_UNIT_TAGS, UNIT_DISPLAY_NAMES, is_player_unit_type, unit_display_name};
pub use unit_types::{
	LRGSLAB, SMLSLAB, UNIT_TYPE_NAMES, UnitCategory, deploy_orders, is_building_type, is_connector_host_type,
	is_ground_cover_type, resting_orders, slab_for_type, unit_type_id, unit_type_name,
};
