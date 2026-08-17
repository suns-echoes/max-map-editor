//! `UnitOrderType` metadata: the canonical order id → name table plus a
//! name → id inverse, mirroring [`super::unit_types`].
//!
//! A decoded [`UnitRecord::orders`](super::types::UnitRecord::orders) is a
//! `UnitOrderType` byte (`enums.hpp`). The save editor's Unit Properties panel
//! exposes it as a picker; these names are the bridge between the stored byte
//! and a human-readable / scriptable token. Names are lowercase hyphen slugs so
//! they double as command arguments (`object-edit orders sentry`).

/// One past the last defined `UnitOrderType` (`ORDER_COUNT_MAX` = `0x20`).
pub const ORDER_END: usize = 0x20;

/// `ORDER_DISABLE` — a unit that has been disabled (by an infiltrator's disable
/// action) sits on this order for `disabled_turns_remaining` turns. In a `V70`
/// save the recoil byte doubles as that countdown iff the order is this value.
pub const ORDER_DISABLE: u8 = 0x1A;

/// `ORDER_AWAIT` (0x00) — the idle order a unit sits on when it has nothing to
/// do; the order a freshly placed/deployed unit is given.
pub const ORDER_AWAIT: u8 = 0x00;

/// `ORDER_BUILD` (0x04) — a builder/factory actively constructing something.
pub const ORDER_BUILD: u8 = 0x04;

/// `ORDER_POWER_ON` (0x07) — a powered building actively running. The engine's
/// deploy path gives a fresh `COMMTWR`/`HABITAT`/`MININGST` this order
/// (`units_manager.cpp` `UnitsManager_DeployUnit`), and a mining station
/// **produces only while on it** (`Cargo_GetNetProduction` gates on the order)
/// — see [`super::unit_types::deploy_orders`].
pub const ORDER_POWER_ON: u8 = 0x07;

/// `ORDER_POWER_OFF` (0x08) — a powered building switched off. The deploy path
/// starts `POWERSTN`/`POWGEN`/`RESEARCH` here; the game powers them up on
/// demand (`GameManager_OptimizeProduction`).
pub const ORDER_POWER_OFF: u8 = 0x08;

/// `ORDER_SENTRY` (0x0C) — hold position and engage on sight. The deploy path
/// gives the defensive fixtures (turrets, laid mines, radar) this order.
pub const ORDER_SENTRY: u8 = 0x0C;

/// `ORDER_IDLE` (0x10) — a unit that is stored / not on the board. The engine's
/// connected-building lookup (`Access_GetTeamBuilding`) skips units on this
/// order, so an idle host never joins a complex through adjacency
/// (`crate::save::complexes`).
pub const ORDER_IDLE: u8 = 0x10;

/// Selected `UnitOrderStateType` values (`enums.hpp`) — the sub-order `state`
/// that pairs with `orders`. A placed/idle unit sits on
/// [`ORDER_STATE_EXECUTING_ORDER`]; a mid-action state such as
/// [`ORDER_STATE_BUILD_IN_PROGRESS`] cannot legitimately coexist with an idle
/// order, which is how the integrity pass (`crate::save::integrity`) recognizes a
/// unit cloned from a mid-build template (`save-editor-bug.md`).
pub const ORDER_STATE_EXECUTING_ORDER: u8 = 0x01;
/// `ORDER_STATE_INIT` — the "just issued, not yet processed" state. A freshly
/// deployed powered host (`ORDER_POWER_ON`) must carry it: the engine's
/// `UnitsManager_ProcessOrderPowerOn` runs `PowerUp` (complex power/resource
/// bookkeeping + the lit sprite frame) **only** from this state, so a
/// power-on unit written at [`ORDER_STATE_EXECUTING_ORDER`] never actually
/// powers up in-game (`units_manager.cpp:2775`). `FileLoad` keeps it as-is
/// (its sanitize list covers only the five in-flight movement states).
pub const ORDER_STATE_INIT: u8 = 0x00;

/// The sub-order state a freshly **deployed** unit pairs with `orders` — the
/// engine's own pairing in `UnitsManager_DeployUnit`: `ORDER_POWER_ON` starts
/// at [`ORDER_STATE_INIT`] (so the first game tick performs the power-up),
/// everything else at the settled [`ORDER_STATE_EXECUTING_ORDER`].
pub fn deploy_state_for(orders: u8) -> u8 {
	if orders == ORDER_POWER_ON { ORDER_STATE_INIT } else { ORDER_STATE_EXECUTING_ORDER }
}
/// A unit part-way through constructing something (`ORDER_STATE_BUILD_IN_PROGRESS`).
pub const ORDER_STATE_BUILD_IN_PROGRESS: u8 = 0x0B;
/// A construction site being chosen (`ORDER_STATE_SELECT_SITE`) — with
/// [`ORDER_STATE_BUILD_CLEARING`], the states a **building under construction**
/// sits in. Such a building legitimately has no `Complex` yet: the engine
/// attaches one at completion (`UnitsManager_UpdateConnectors` →
/// `AttachToPrimaryComplex`), and DOS-era stock scenarios ship saves mid-build
/// (`crate::save::complexes`).
pub const ORDER_STATE_SELECT_SITE: u8 = 0x19;
/// The construction site being cleared (`ORDER_STATE_BUILD_CLEARING`).
pub const ORDER_STATE_BUILD_CLEARING: u8 = 0x1A;

/// `UnitOrderType` slugs for every order `0x00..ORDER_END`, in enum order
/// (`enums.hpp`). Indexed directly by the stored `orders` byte. `ORDER_17`
/// (`0x12`) is undocumented in the engine and kept as a positional placeholder.
pub const ORDER_NAMES: [&str; ORDER_END] = [
	"await",           // 0x00 ORDER_AWAIT
	"transform",       // 0x01 ORDER_TRANSFORM
	"move",            // 0x02 ORDER_MOVE
	"fire",            // 0x03 ORDER_FIRE
	"build",           // 0x04 ORDER_BUILD
	"activate",        // 0x05 ORDER_ACTIVATE
	"allocate",        // 0x06 ORDER_NEW_ALLOCATE
	"power-on",        // 0x07 ORDER_POWER_ON
	"power-off",       // 0x08 ORDER_POWER_OFF
	"explode",         // 0x09 ORDER_EXPLODE
	"unload",          // 0x0A ORDER_UNLOAD
	"clear",           // 0x0B ORDER_CLEAR
	"sentry",          // 0x0C ORDER_SENTRY
	"land",            // 0x0D ORDER_LAND
	"take-off",        // 0x0E ORDER_TAKE_OFF
	"load",            // 0x0F ORDER_LOAD
	"idle",            // 0x10 ORDER_IDLE
	"repair",          // 0x11 ORDER_REPAIR
	"order-17",        // 0x12 ORDER_17 (undocumented)
	"reload",          // 0x13 ORDER_RELOAD
	"transfer",        // 0x14 ORDER_TRANSFER
	"halt-building",   // 0x15 ORDER_HALT_BUILDING
	"await-scaling",   // 0x16 ORDER_AWAIT_SCALING
	"await-tape",      // 0x17 ORDER_AWAIT_TAPE_POSITIONING
	"await-steal",     // 0x18 ORDER_AWAIT_STEAL_UNIT
	"await-disable",   // 0x19 ORDER_AWAIT_DISABLE_UNIT
	"disable",         // 0x1A ORDER_DISABLE
	"move-to-unit",    // 0x1B ORDER_MOVE_TO_UNIT
	"upgrade",         // 0x1C ORDER_UPGRADE
	"lay-mine",        // 0x1D ORDER_LAY_MINE
	"move-to-attack",  // 0x1E ORDER_MOVE_TO_ATTACK
	"halt-building-2", // 0x1F ORDER_HALT_BUILDING_2
];

/// The slug for an order byte, or `None` for a value at/above [`ORDER_END`]
/// (there are none in stock saves, but a raw byte can hold anything).
pub fn order_name(order: u8) -> Option<&'static str> {
	ORDER_NAMES.get(order as usize).copied()
}

/// The order byte for a slug (case-insensitive), the inverse of [`order_name`];
/// `None` for an unknown token.
pub fn order_id(name: &str) -> Option<u8> {
	ORDER_NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)).map(|i| i as u8)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_and_ids_round_trip() {
		for id in 0..ORDER_END as u8 {
			let name = order_name(id).expect("named");
			assert_eq!(order_id(name), Some(id), "{name} round-trips");
		}
	}

	#[test]
	fn known_orders_have_expected_ids() {
		assert_eq!(order_id("await"), Some(0x00));
		assert_eq!(order_id("build"), Some(0x04));
		assert_eq!(order_id("sentry"), Some(0x0C));
		assert_eq!(order_id("disable"), Some(0x1A));
		assert_eq!(order_name(0x10), Some("idle"));
	}

	#[test]
	fn parse_is_case_insensitive_and_bounded() {
		assert_eq!(order_id("SENTRY"), Some(0x0C));
		assert_eq!(order_id("Sentry"), Some(0x0C));
		assert_eq!(order_id("nonsense"), None);
		assert_eq!(order_name(0xFF), None);
	}
}
