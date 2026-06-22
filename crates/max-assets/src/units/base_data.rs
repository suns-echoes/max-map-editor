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
