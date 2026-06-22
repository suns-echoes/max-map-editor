//! Small shared helpers over a `w`×`h` row-major cell grid.

/// 4-connected flood fill over a `w`×`h` row-major grid from index `start`.
///
/// `seen` is the caller-owned visited set: that lets connected-components
/// labeling share one buffer across many floods (each cell is entered once
/// across the whole sweep), while a one-shot caller just hands in a fresh
/// `vec![false; w * h]`. `flood4` marks `start` and every cell it reaches; a
/// neighbour `j` is entered iff `!seen[j] && pred(j)` (so `pred` is the
/// "belongs to the region" test), and `visit(i)` runs once per reached cell,
/// in pop order.
pub(crate) fn flood4(
	w: usize,
	h: usize,
	start: usize,
	seen: &mut [bool],
	mut pred: impl FnMut(usize) -> bool,
	mut visit: impl FnMut(usize),
) {
	let mut stack = vec![start];
	seen[start] = true;
	while let Some(i) = stack.pop() {
		visit(i);
		let (x, y) = (i % w, i / w);
		for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
			if nx >= w || ny >= h {
				continue;
			}
			let j = ny * w + nx;
			if !seen[j] && pred(j) {
				seen[j] = true;
				stack.push(j);
			}
		}
	}
}
