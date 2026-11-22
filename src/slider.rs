const OTHER_COLUMNS: u16 = 33;

const WIDE: u16 = 40;

const USUAL: u16 = 20;

const NARROW: u16 = 10;

const TITLE_BEFORE_WIDE: u16 = 30;

pub fn width_for(window: u16) -> u16 {
    let room = window.saturating_sub(OTHER_COLUMNS);
    if room >= WIDE + TITLE_BEFORE_WIDE {
        WIDE
    } else if room >= USUAL {
        USUAL
    } else {
        NARROW
    }
}

fn is_tick(width: u16, i: u16) -> bool {
    if i >= width {
        return false;
    }
    let scaled = u32::from(i) * 100;
    let width = u32::from(width);
    scaled % width == 0 && (scaled / width) % 5 == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    EmptyTick,
    Edge(u8),
    Lit,
    LitTick,
    Lead,
    LeadTick,
}

pub fn eighths(volume: f32, width: u16) -> usize {
    (volume.clamp(0.0, 1.0) * f32::from(width) * 8.0).round() as usize
}

pub fn cells(volume: f32, width: u16) -> impl Iterator<Item = Cell> {
    let reach = eighths(volume, width);
    let lead = reach.div_ceil(8).checked_sub(1);
    (0..width).map(move |i| {
        let tick = is_tick(width, i);
        let leads = lead == Some(usize::from(i));
        match reach.saturating_sub(usize::from(i) * 8).min(8) {
            0 if tick => Cell::EmptyTick,
            0 => Cell::Empty,
            8 if leads && tick => Cell::LeadTick,
            8 if leads => Cell::Lead,
            8 if tick => Cell::LitTick,
            8 => Cell::Lit,
            n => Cell::Edge(n as u8),
        }
    })
}

pub fn volume_at(bar_x: u16, width: u16, column: u16) -> f32 {
    if width == 0 || column < bar_x {
        return 0.0;
    }
    let width = u32::from(width);
    let lit = (u32::from(column - bar_x) + 1).min(width);
    ((lit * 100 + width / 2) / width) as f32 / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks(width: u16) -> Vec<u16> {
        (0..width).filter(|&i| is_tick(width, i)).collect()
    }

    fn percent(volume: f32) -> u32 {
        (volume * 100.0).round() as u32
    }

    fn leads(c: Cell) -> bool {
        matches!(c, Cell::Edge(_) | Cell::Lead | Cell::LeadTick)
    }

    #[test]
    fn the_slider_is_twenty_cells_or_forty_when_the_window_has_room() {
        assert_eq!(width_for(0), 10);
        assert_eq!(width_for(30), 10);
        assert_eq!(width_for(52), 10);
        assert_eq!(width_for(53), 20);
        assert_eq!(width_for(80), 20);
        assert_eq!(width_for(102), 20);
        assert_eq!(width_for(103), 40);
        assert_eq!(width_for(400), 40);
        for window in 103..400u16 {
            assert!(window - OTHER_COLUMNS - width_for(window) >= TITLE_BEFORE_WIDE);
        }
    }

    #[test]
    fn there_is_a_tick_at_every_five_percent_in_the_usual_and_the_full_slider() {
        assert_eq!(ticks(USUAL), (0..20).collect::<Vec<u16>>());
        assert_eq!(ticks(WIDE), (0..20).map(|k| k * 2).collect::<Vec<u16>>());
        for width in [USUAL, WIDE] {
            for (n, &cell) in ticks(width).iter().enumerate() {
                let percent = n as u32 * 5;
                assert_eq!(u32::from(cell) * 100, percent * u32::from(width), "width {width}, tick {n}");
            }
        }
    }

    #[test]
    fn the_narrow_slider_marks_every_ten_percent() {
        assert_eq!(ticks(NARROW), (0..10).collect::<Vec<u16>>());
    }

    #[test]
    fn a_width_that_cannot_hold_whole_marks_gets_only_the_one_at_the_start() {
        assert_eq!(ticks(13), vec![0]);
        assert!(ticks(0).is_empty());
        assert_eq!(ticks(1), vec![0]);
    }

    #[test]
    fn one_percent_always_moves_the_fill() {
        for width in [USUAL, WIDE] {
            let reach: Vec<usize> = (0..=100).map(|p| eighths(p as f32 / 100.0, width)).collect();
            assert_eq!(reach[0], 0);
            assert_eq!(reach[100], usize::from(width) * 8);
            for p in 1..=100 {
                assert!(reach[p] > reach[p - 1], "width {width}: {p}% looks the same as {}%", p - 1);
            }
        }
    }

    #[test]
    fn at_every_multiple_of_five_the_fill_ends_exactly_on_its_tick() {
        for width in [USUAL, WIDE] {
            for (n, &tick) in ticks(width).iter().enumerate().skip(1) {
                let volume = n as f32 * 5.0 / 100.0;
                assert_eq!(eighths(volume, width), usize::from(tick) * 8, "width {width}, {}%", n * 5);
                let drawn: Vec<Cell> = cells(volume, width).collect();
                let (before, rest) = drawn.split_at(usize::from(tick));
                let (last, body) = before.split_last().unwrap();
                assert!(body.iter().all(|c| matches!(c, Cell::Lit | Cell::LitTick)), "width {width}: {drawn:?}");
                assert!(matches!(last, Cell::Lead | Cell::LeadTick), "width {width}: the cell the fill ends in leads: {drawn:?}");
                assert_eq!(rest[0], Cell::EmptyTick, "width {width}: the tick starts the bare track");
            }
        }
    }

    #[test]
    fn the_fill_ends_part_way_through_a_cell() {
        let c: Vec<Cell> = cells(0.52, 20).collect();
        assert_eq!(c.len(), 20);
        assert_eq!(c[0], Cell::LitTick, "the left end has its tick like every other cell");
        assert_eq!(c[9], Cell::LitTick);
        assert_eq!(c[10], Cell::Edge(3), "the 50% tick is under the fill: its end is the mark");
        assert_eq!(c[11], Cell::EmptyTick);
        assert_eq!(c[19], Cell::EmptyTick);
    }

    #[test]
    fn nothing_is_lit_at_zero_and_everything_at_full_whatever_the_input() {
        let bare = |c: Cell| matches!(c, Cell::Empty | Cell::EmptyTick);
        let lit = |c: Cell| matches!(c, Cell::Lit | Cell::LitTick | Cell::Lead | Cell::LeadTick);
        for width in [NARROW, USUAL, WIDE] {
            assert!(cells(0.0, width).all(bare));
            assert!(cells(1.0, width).all(lit));
            assert!(cells(-3.0, width).all(bare));
            assert!(cells(7.0, width).all(lit));
            assert!(cells(f32::NAN, width).all(bare));
        }
    }

    #[test]
    fn a_click_fills_the_slider_up_to_the_cell_it_hit() {
        assert_eq!(percent(volume_at(50, 20, 50)), 5);
        assert_eq!(percent(volume_at(50, 20, 59)), 50);
        assert_eq!(percent(volume_at(50, 20, 69)), 100, "the last cell is the top");
        assert_eq!(percent(volume_at(50, 20, 70)), 100, "past the end is the top");
        assert_eq!(percent(volume_at(50, 20, 500)), 100);
        assert_eq!(percent(volume_at(50, 20, 49)), 0, "left of the slider is 0");
        assert_eq!(percent(volume_at(50, 20, 0)), 0);
        assert_eq!(percent(volume_at(50, 40, 50)), 3);
        assert_eq!(percent(volume_at(50, 40, 51)), 5);
        assert_eq!(percent(volume_at(50, 40, 89)), 100);
        assert_eq!(percent(volume_at(50, 10, 50)), 10);
        assert_eq!(percent(volume_at(50, 10, 59)), 100);
        assert_eq!(volume_at(50, 0, 60), 0.0);
    }

    #[test]
    fn the_cell_just_before_a_tick_gives_exactly_that_ticks_percent() {
        for width in [USUAL, WIDE] {
            for (n, &tick) in ticks(width).iter().enumerate().skip(1) {
                let got = percent(volume_at(30, width, 30 + tick - 1));
                assert_eq!(got, n as u32 * 5, "width {width}, tick {n}");
            }
        }
    }

    #[test]
    fn moving_right_never_lowers_the_volume() {
        for width in [NARROW, USUAL, WIDE] {
            let mut last = 0.0;
            for column in 0..200 {
                let v = volume_at(20, width, column);
                assert!(v >= last, "width {width}, column {column}");
                last = v;
            }
        }
    }

    #[test]
    fn ticks_show_inside_and_past_the_fill_but_never_on_its_end() {
        let c: Vec<Cell> = cells(0.12, 20).collect();
        assert_eq!(c[0], Cell::LitTick, "0%");
        assert_eq!(c[1], Cell::LitTick, "5%");
        assert_eq!(c[2], Cell::Edge(3), "10% falls where the fill ends");
        assert_eq!(c[3], Cell::EmptyTick, "15%");
        assert_eq!(c[19], Cell::EmptyTick);
    }

    #[test]
    fn exactly_one_bar_leads_and_it_is_the_cell_the_fill_ends_in() {
        for width in [NARROW, USUAL, WIDE] {
            for p in 0..=100u32 {
                let drawn: Vec<Cell> = cells(p as f32 / 100.0, width).collect();
                let leading: Vec<usize> = (0..drawn.len()).filter(|&i| leads(drawn[i])).collect();
                if p == 0 {
                    assert!(leading.is_empty(), "width {width}: nothing leads at 0%");
                    continue;
                }
                let end = drawn.iter().rposition(|c| !matches!(c, Cell::Empty | Cell::EmptyTick)).unwrap();
                assert_eq!(leading, vec![end], "width {width}, {p}%: {drawn:?}");
                for c in &drawn[..end] {
                    assert!(matches!(c, Cell::Lit | Cell::LitTick), "width {width}, {p}%: {drawn:?}");
                }
                assert!(drawn[end + 1..].iter().all(|c| matches!(c, Cell::Empty | Cell::EmptyTick)), "width {width}, {p}%");
            }
        }
    }

    #[test]
    fn the_first_bar_is_made_like_the_ones_after_it() {
        for width in [NARROW, USUAL, WIDE] {
            let step = usize::from(width) / if width == NARROW { 10 } else { 20 };
            let lit: Vec<Cell> = cells(0.8, width).collect();
            assert_eq!(lit[0], Cell::LitTick, "width {width}");
            assert_eq!(lit[step], lit[0], "width {width}");
            let bare: Vec<Cell> = cells(0.0, width).collect();
            assert_eq!(bare[0], Cell::EmptyTick, "width {width}");
            assert_eq!(bare[step], bare[0], "width {width}");
        }
    }
}
