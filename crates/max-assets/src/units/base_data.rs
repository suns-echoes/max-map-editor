/// `D_*` file payload - indexes into the unit's sprite strip for each
/// category (body, turret, firing, connector). Original MAX reads 8 bytes
/// plus 8 angle offsets (16 bytes signed `i8` pairs). For now only the
/// base/count section is decoded - angle offsets TODO.
#[derive(Debug, Clone, Copy, Default)]
pub struct BaseUnitData {
	pub image_base: u8,
	pub image_count: u8,
	pub turret_image_base: u8,
	pub turret_image_count: u8,
	pub firing_image_base: u8,
	pub firing_image_count: u8,
	pub connector_image_base: u8,
	pub connector_image_count: u8,
}

impl BaseUnitData {
	pub const SIZE: usize = 8;

	pub fn from_bytes(data: &[u8]) -> Option<Self> {
		if data.len() < Self::SIZE {
			return None;
		}
		Some(BaseUnitData {
			image_base: data[0],
			image_count: data[1],
			turret_image_base: data[2],
			turret_image_count: data[3],
			firing_image_base: data[4],
			firing_image_count: data[5],
			connector_image_base: data[6],
			connector_image_count: data[7],
		})
	}
}

pub fn parse_base_unit_data(data: &[u8]) -> Option<BaseUnitData> {
	BaseUnitData::from_bytes(data)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The eight header bytes map positionally onto the four (base, count)
	/// pairs: body, turret, firing, connector.
	#[test]
	fn from_bytes_maps_the_eight_bytes_in_order() {
		let d = parse_base_unit_data(&[1, 2, 3, 4, 5, 6, 7, 8]).expect("8 bytes is exactly BaseUnitData::SIZE");
		assert_eq!((d.image_base, d.image_count), (1, 2), "body sprite range");
		assert_eq!((d.turret_image_base, d.turret_image_count), (3, 4), "turret sprite range");
		assert_eq!((d.firing_image_base, d.firing_image_count), (5, 6), "firing sprite range");
		assert_eq!((d.connector_image_base, d.connector_image_count), (7, 8), "connector sprite range");
	}

	/// Payloads shorter than `SIZE` are refused; longer ones decode from the
	/// first 8 bytes (D_* files carry angle offsets after them - not decoded
	/// yet, and they must not confuse the parser).
	#[test]
	fn short_input_is_rejected_and_extra_bytes_are_ignored() {
		assert!(BaseUnitData::from_bytes(&[]).is_none(), "an empty payload has no header");
		assert!(BaseUnitData::from_bytes(&[0; BaseUnitData::SIZE - 1]).is_none(), "one byte short of SIZE");
		let with_tail = parse_base_unit_data(&[9; 24]).expect("a trailing angle-offset block is fine");
		assert_eq!(with_tail.image_base, 9, "fields still come from the leading 8 bytes");
	}
}
