use crossterm::event::{Event, KeyCode};
use ratatui::{
	style::Style,
	widgets::{Axis, GraphType},
};

use crate::scope_tui::SampleTape;

use super::{DataSet, Dimension, DisplayMode, GraphConfig};

#[derive(Default)]
pub struct Vectorscope {
	pub goniometer: bool,
}

impl DisplayMode for Vectorscope {
	fn channel_name(&self, index: usize) -> String {
		format!("{index}")
	}

	fn axis(&'_ self, cfg: &GraphConfig, _: Dimension) -> Axis<'_> {
		Axis::default().style(Style::default().fg(cfg.axis_color)).bounds([-cfg.scale, cfg.scale])
	}

	fn references(&self, _cfg: &GraphConfig) -> Vec<DataSet> {
		vec![]
	}

	fn process(&mut self, cfg: &GraphConfig, data: &SampleTape) -> Vec<DataSet> {
		let mut out = Vec::new();

		for (n, chunk) in data.chunks(2).enumerate() {
			let mut tmp = vec![];
			match chunk.len() {
				2 => {
					for i in 0..std::cmp::min(chunk[0].len(), chunk[1].len()) {
						if i > cfg.samples as usize {
							break;
						}
						if self.goniometer {
							tmp.push((mid(chunk[0][i], chunk[1][i]), side(chunk[0][i], chunk[1][i])));
						} else {
							tmp.push((chunk[0][i], chunk[1][i]));
						}
					}
				}
				1 => {
					for i in 0..chunk[0].len() {
						if i > cfg.samples as usize {
							break;
						}
						tmp.push((chunk[0][i], chunk[0][i]));
					}
				}
				_ => continue,
			}
			let pivot = tmp.len() / 2;
			out.push(DataSet::new(
				Some(self.channel_name((n * 2) + 1)),
				tmp[pivot..].to_vec(),
				cfg.marker_type,
				if cfg.scatter {
					GraphType::Scatter
				} else {
					GraphType::Line
				},
				cfg.palette((n * 2) + 1),
			));
			out.push(DataSet::new(
				Some(self.channel_name(n * 2)),
				tmp[..pivot].to_vec(),
				cfg.marker_type,
				if cfg.scatter {
					GraphType::Scatter
				} else {
					GraphType::Line
				},
				cfg.palette(n * 2),
			));
		}

		out
	}

	fn handle(&mut self, event: Event) {
		if let Event::Key(key) = event {
			match key.code {
				KeyCode::Char('g') => self.goniometer = !self.goniometer,
				_ => {}
			}
		}
	}
}

fn mid(a: f64, b: f64) -> f64 {
	if a > 0. && b > 0. {
		a.min(b)
	} else if a < 0. && b < 0. {
		a.max(b)
	} else {
		0.
	}
}

fn side(a: f64, b: f64) -> f64 {
	a - b
}
