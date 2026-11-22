#[rustfmt::skip]
#[derive(Debug, PartialEq, Clone)]
pub enum Tone {
	C, Db, D, Eb, E, F, Gb, G, Ab, A, Bb, B,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Note {
	tone: Tone,
	octave: u32,
}

impl Note {
	pub fn from_frequency(freq: f32) -> Option<(Note, f32)> {
		if !freq.is_finite() || freq < 16.0 {
			return None;
		}
		let midi = 69.0 + 12.0 * (freq / 440.0).log2();
		let nearest = midi.round();
		let cents = (midi - nearest) * 100.0;
		let n = nearest as i32;
		if n < 12 {
			return None;
		}
		#[rustfmt::skip]
		let tones = [Tone::C, Tone::Db, Tone::D, Tone::Eb, Tone::E, Tone::F, Tone::Gb, Tone::G, Tone::Ab, Tone::A, Tone::Bb, Tone::B];
		let tone = tones[(n % 12) as usize].clone();
		Some((Note { tone, octave: (n / 12 - 1) as u32 }, cents))
	}
}

impl std::fmt::Display for Note {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let name = match self.tone {
			Tone::C => "C", Tone::Db => "C#", Tone::D => "D", Tone::Eb => "D#", Tone::E => "E", Tone::F => "F",
			Tone::Gb => "F#", Tone::G => "G", Tone::Ab => "G#", Tone::A => "A", Tone::Bb => "A#", Tone::B => "B",
		};
		write!(f, "{}{}", name, self.octave)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn notes_from_frequencies() {
		let (n, c) = Note::from_frequency(440.0).unwrap();
		assert_eq!((n.to_string(), c.round()), ("A4".to_string(), 0.0));
		let (n, c) = Note::from_frequency(110.0 * 2f32.powf(-4.0 / 1200.0)).unwrap();
		assert_eq!((n.to_string(), c.round()), ("A2".to_string(), -4.0));
		assert_eq!(Note::from_frequency(261.63).unwrap().0.to_string(), "C4");
		assert_eq!(Note::from_frequency(277.18).unwrap().0.to_string(), "C#4");
		assert!(Note::from_frequency(5.0).is_none());
	}
}
