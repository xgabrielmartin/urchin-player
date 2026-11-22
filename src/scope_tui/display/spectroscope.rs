use std::collections::VecDeque;

use crossterm::event::{Event, KeyCode};
use ratatui::{
	style::Style,
	widgets::{Axis, GraphType},
};

use crate::scope_tui::SampleTape;

use super::{update_value_i, DataSet, Dimension, DisplayMode, GraphConfig};

use realfft::{num_complex::Complex, RealFftPlanner};

pub struct Spectroscope {
	pub sampling_rate: u32,
	pub buffer_size: u32,
	pub average: u32,
	pub buf: Vec<VecDeque<Vec<f64>>>,
	pub window: bool,
	pub log_y: bool,
	pub phase_diff: bool,
	planner: RealFftPlanner<f64>,
	hann: Vec<f64>,
	fft_in: Vec<f64>,
	fft_buf: Vec<Complex<f64>>,
	scratch: Vec<Complex<f64>>,
}

fn magnitude(c: Complex<f64>) -> f64 {
	let squared = (c.re * c.re) + (c.im * c.im);
	squared.sqrt()
}

fn phase(c: Complex<f64>) -> f64 {
	let r = f64::atan2(c.re, c.im);
	if r.is_normal() { r } else { 0. }
}

pub fn hann_window(samples: &[f64]) -> Vec<f64> {
	let mut windowed_samples = Vec::with_capacity(samples.len());
	let samples_len = samples.len() as f64;
	for (i, sample) in samples.iter().enumerate() {
		let two_pi_i = 2.0 * std::f64::consts::PI * i as f64;
		let idontknowthename = (two_pi_i / samples_len).cos();
		let multiplier = 0.5 * (1.0 - idontknowthename);
		windowed_samples.push(sample * multiplier);
	}
	windowed_samples
}

impl Spectroscope {
	pub fn new(sampling_rate: u32, buffer_size: u32) -> Self {
		Spectroscope {
			sampling_rate,
			buffer_size,
			average: 10,
			buf: Vec::new(),
			window: true,
			log_y: true,
			phase_diff: false,
			planner: RealFftPlanner::new(),
			hann: Vec::new(),
			fft_in: Vec::new(),
			fft_buf: Vec::new(),
			scratch: Vec::new(),
		}
	}
}

impl DisplayMode for Spectroscope {
	fn channel_name(&self, index: usize) -> String {
		match index {
			0 => "L".into(),
			1 => "R".into(),
			_ => format!("{index}"),
		}
	}

	fn axis(&'_ self, cfg: &GraphConfig, dimension: Dimension) -> Axis<'_> {
		let bounds = match dimension {
			Dimension::X => [
				20.0f64.ln(),
				((cfg.samples as f64 / (cfg.initial_samples * cfg.chunks) as f64) * 20000.0).ln(),
			],
			Dimension::Y => [0.0, cfg.scale * 7.5],
		};
		Axis::default().style(Style::default().fg(cfg.axis_color)).bounds(bounds)
	}

	fn process(&mut self, cfg: &GraphConfig, data: &SampleTape) -> Vec<DataSet> {
		if self.average == 0 {
			self.average = 1;
		}
		if !cfg.pause {
			for (i, chan) in data.iter().enumerate() {
				if self.buf.len() <= i {
					self.buf.push(VecDeque::new());
				}
				self.buf[i].push_back(chan.clone());
				while self.buf[i].len() > self.average as usize {
					self.buf[i].pop_front();
				}
			}
		}

		let mut out = Vec::new();
		let sample_len = self.buffer_size * self.average;
		let resolution = self.sampling_rate as f64 / sample_len as f64;
		let fft = self.planner.plan_fft_forward(sample_len as usize);
		let bins = sample_len as usize / 2 + 1;

		let mut all_phases = Vec::new();

		for (n, chan_queue) in self.buf.iter().enumerate().rev() {
			let mut phases = Vec::new();
			let len = chan_queue.iter().map(Vec::len).sum::<usize>();
			if self.window && self.hann.len() != len {
				self.hann = hann_window(&vec![1.0; len]);
			}
			let (name, phase_diff, log_y) = (Some(self.channel_name(n)), self.phase_diff, self.log_y);
			let input = &mut self.fft_in;
			input.clear();
			input.extend(chan_queue.iter().flatten().enumerate().map(|(i, &x)| if self.window { x * self.hann[i] } else { x }));
			let mut max_val = *input
				.iter()
				.max_by(|a, b| a.total_cmp(b))
				.expect("empty dataset?");
			if max_val < 1. {
				max_val = 1.;
			}
			for x in input.iter_mut() {
				*x /= max_val;
			}
			input.resize(sample_len as usize, 0.0);
			let tmp = &mut self.fft_buf;
			tmp.resize(fft.complex_len(), Complex { re: 0.0, im: 0.0 });
			self.scratch.resize(fft.get_scratch_len(), Complex { re: 0.0, im: 0.0 });
			fft.process_with_scratch(input, tmp, &mut self.scratch).expect("buffers sized for this plan");
			out.push(DataSet::new(
				name,
				tmp
					.iter()
					.take(bins)
					.enumerate()
					.map(|(i, x)| {
						if phase_diff {
							phases.push(phase(*x));
						}
						(
							(i as f64 * resolution).ln(),
							if log_y {
								magnitude(*x).ln()
							} else {
								magnitude(*x)
							},
						)
					})
					.collect(),
				cfg.marker_type,
				if cfg.scatter {
					GraphType::Scatter
				} else {
					GraphType::Line
				},
				cfg.palette(n),
			));
			all_phases.push(phases);
		}

		if self.phase_diff {
			let mut phase_diff = Vec::new();
			for i in 0..all_phases.first().map(|x| x.len()).unwrap_or_default() {
				for p in all_phases.chunks(2) {
					phase_diff.push((
						(i as f64 * resolution).ln(),
						(p[0].get(i).copied().unwrap_or_default() - p[1].get(i).copied().unwrap_or_default()).abs()
					));
				}
			}

			out.insert(0, DataSet::new(
				Some("phase diff".to_string()),
				phase_diff,
				cfg.marker_type,
				GraphType::Scatter,
				cfg.palette(self.buf.len()),
			));
		}

		out
	}

	fn handle(&mut self, event: Event) {
		if let Event::Key(key) = event {
			match key.code {
				KeyCode::PageUp => update_value_i(&mut self.average, true, 1, 1., 1..65535),
				KeyCode::PageDown => update_value_i(&mut self.average, false, 1, 1., 1..65535),
				KeyCode::Char('w') => self.window = !self.window,
				KeyCode::Char('l') => self.log_y = !self.log_y,
				KeyCode::Char('p') => self.phase_diff = !self.phase_diff,
				_ => {}
			}
		}
	}

	fn references(&self, _cfg: &GraphConfig) -> Vec<DataSet> {
		vec![]
	}
}
