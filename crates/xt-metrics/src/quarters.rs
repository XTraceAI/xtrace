//! Agent time (M-05) by local wall-clock quarter-hour: for each local day,
//! the 96 quarters from 00:00 to 24:00 on that day's clock, each holding the
//! part of every agent span that falls in it, parallel sessions adding as
//! M-05 defines. Nothing is re-read and no span is re-folded: the spans
//! [`ActiveSpanReport::by_day`] splits at midnight are split here at every
//! quarter-hour too, by the same allocation, so a day's quarters add up to
//! exactly that day's agent time.
//!
//! Clock changes are decided here, once. When the clock falls back, the
//! repeated wall-clock quarters hold the time of both passes: their
//! `elapsed_ms` is 30 minutes, so the average number of agents running is
//! still agent time over real time. When the clock springs forward, the
//! skipped quarters stay empty with `elapsed_ms` 0.
use crate::{ActiveSpanReport, DayActive, Error, Result, Window, window::allocate};
use jiff::{Timestamp, civil::Time, tz::TimeZone};
use serde::Serialize;

/// Wall-clock quarter-hours in a local day, 00:00 to 24:00.
pub const QUARTERS_PER_DAY: usize = 96;
/// One quarter-hour of wall-clock time.
pub const QUARTER_MS: i64 = 15 * 60_000;

/// One local wall-clock quarter-hour of one day.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuarterHour {
    /// Agent time in this quarter, parallel sessions adding.
    pub agent_ms: u64,
    /// Real time this wall-clock quarter covered on its day: 15 minutes, 30
    /// when a fall-back clock change repeats it, 0 when a spring-forward
    /// change skips it. The average number of agents running is
    /// `agent_ms / elapsed_ms`; the app works it out once, at the precision
    /// it shows.
    pub elapsed_ms: u64,
}

/// One local day's agent time and its 96 wall-clock quarters.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayQuarters {
    /// The day and its agent time, equal to [`ActiveSpanReport::by_day`]'s.
    pub day: DayActive,
    /// Exactly [`QUARTERS_PER_DAY`] quarters, from 00:00; they add up to
    /// `day.active_ms`.
    pub quarters: Vec<QuarterHour>,
    /// The first quarter with the most agents running on average; none when
    /// the day has no agent time.
    pub busiest: Option<u8>,
}

/// One piece of the window: a stretch of real time inside one day that sits
/// in one wall-clock quarter.
struct Piece {
    end_ms: i64,
    length_ms: i64,
    day: usize,
    quarter: usize,
}

/// Split the window's local days at every wall-clock quarter-hour, in order,
/// covering the window exactly.
fn pieces(window: Window, zone: &TimeZone) -> Result<(Vec<crate::DayBucket>, Vec<Piece>)> {
    let days = window.local_days(zone.clone())?;
    let mut out = Vec::new();
    for (index, bucket) in days.iter().enumerate() {
        let (start, end) = (bucket.window.start_ms(), bucket.window.end_ms());
        // The day's 00:00 written as if it were UTC: wall-clock time on this
        // day is an instant plus its offset, minus this.
        let naive_midnight = bucket
            .date
            .to_datetime(Time::midnight())
            .to_zoned(TimeZone::UTC)?
            .timestamp()
            .as_millisecond();
        // The day in stretches of one constant offset.
        let mut cuts = vec![start];
        for transition in zone.following(Timestamp::from_millisecond(start)?) {
            let at = transition.timestamp().as_millisecond();
            if at >= end {
                break;
            }
            if at > start {
                cuts.push(at);
            }
        }
        cuts.push(end);
        for pair in cuts.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let offset_ms =
                i64::from(zone.to_offset(Timestamp::from_millisecond(from)?).seconds()) * 1000;
            // Real instants of this stretch's quarter-hour boundaries.
            let mut at = from;
            while at < to {
                let wall = at + offset_ms - naive_midnight;
                let quarter = wall.div_euclid(QUARTER_MS);
                let next_boundary = naive_midnight + (quarter + 1) * QUARTER_MS - offset_ms;
                let stop = next_boundary.min(to);
                if stop <= at {
                    return Err(Error::InvalidWindow);
                }
                // A wall clock that leaves the date (an offset change across
                // midnight) keeps its time in the nearest quarter of the day.
                let quarter = quarter.clamp(0, QUARTERS_PER_DAY as i64 - 1) as usize;
                out.push(Piece {
                    end_ms: stop,
                    length_ms: stop - at,
                    day: index,
                    quarter,
                });
                at = stop;
            }
        }
    }
    Ok((days, out))
}

impl ActiveSpanReport {
    /// [`ActiveSpanReport::by_day`] with each day also split into its 96 local
    /// wall-clock quarter-hours. The same allocation splits the same spans
    /// at finer boundaries, so every day's quarters add up to its agent time
    /// and the days to the report's.
    pub fn by_quarter_hour(&self, window: Window, zone: TimeZone) -> Result<Vec<DayQuarters>> {
        let (days, pieces) = pieces(window, &zone)?;
        let ends: Vec<i128> = pieces.iter().map(|p| i128::from(p.end_ms)).collect();
        let mut totals = vec![0_i128; pieces.len()];
        let start_of_window = i128::from(window.start_ms());
        for span in &self.spans {
            let start = i128::from(span.start_ms).max(start_of_window);
            // Skip the pieces that end before the span starts; the last piece
            // stays last, so it still takes any remainder past the window.
            let first = ends.partition_point(|end| *end <= start);
            let first = first.min(ends.len() - 1);
            allocate(
                start_of_window,
                &ends[first..],
                (i128::from(span.start_ms), i128::from(span.end_ms)),
                &mut totals[first..],
            )?;
        }
        let mut agent = vec![[0_u64; QUARTERS_PER_DAY]; days.len()];
        let mut elapsed = vec![[0_u64; QUARTERS_PER_DAY]; days.len()];
        for (piece, total) in pieces.iter().zip(&totals) {
            let total = u64::try_from(*total).map_err(|_| Error::CounterOverflow)?;
            let cell = &mut agent[piece.day][piece.quarter];
            *cell = cell.checked_add(total).ok_or(Error::CounterOverflow)?;
            let length = u64::try_from(piece.length_ms).map_err(|_| Error::InvalidWindow)?;
            elapsed[piece.day][piece.quarter] += length;
        }
        days.iter()
            .zip(agent.iter().zip(&elapsed))
            .map(|(bucket, (agent, elapsed))| {
                let quarters: Vec<QuarterHour> = agent
                    .iter()
                    .zip(elapsed)
                    .map(|(&agent_ms, &elapsed_ms)| QuarterHour {
                        agent_ms,
                        elapsed_ms,
                    })
                    .collect();
                let active_ms = quarters
                    .iter()
                    .try_fold(0_u64, |sum, q| sum.checked_add(q.agent_ms))
                    .ok_or(Error::CounterOverflow)?;
                Ok(DayQuarters {
                    day: DayActive {
                        date: bucket.date.to_string(),
                        start_ms: bucket.window.start_ms(),
                        end_ms: bucket.window.end_ms(),
                        active_ms,
                    },
                    busiest: busiest(&quarters),
                    quarters,
                })
            })
            .collect()
    }
}

/// The first quarter with the highest average, compared exactly as
/// `agent_ms / elapsed_ms`; none when no quarter has agent time.
fn busiest(quarters: &[QuarterHour]) -> Option<u8> {
    let mut best: Option<usize> = None;
    for (index, quarter) in quarters.iter().enumerate() {
        if quarter.agent_ms == 0 {
            continue;
        }
        let higher = best.is_none_or(|b| {
            let top = &quarters[b];
            u128::from(quarter.agent_ms) * u128::from(top.elapsed_ms)
                > u128::from(top.agent_ms) * u128::from(quarter.elapsed_ms)
        });
        if higher {
            best = Some(index);
        }
    }
    best.and_then(|index| u8::try_from(index).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActiveSpan;

    const M: i64 = 60_000;

    fn ms(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }
    fn report(spans: &[(i64, i64)]) -> ActiveSpanReport {
        let spans: Vec<ActiveSpan> = spans
            .iter()
            .enumerate()
            .map(|(index, (start_ms, end_ms))| ActiveSpan {
                session_id: format!("s{index}"),
                host: "claude".into(),
                start_ms: *start_ms,
                end_ms: *end_ms,
            })
            .collect();
        let active_ms = spans.iter().map(|s| (s.end_ms - s.start_ms) as u64).sum();
        ActiveSpanReport { spans, active_ms }
    }
    /// Quarters and days must agree with `by_day` exactly.
    fn quarters(report: &ActiveSpanReport, window: Window, zone: TimeZone) -> Vec<DayQuarters> {
        let days = report.by_quarter_hour(window, zone.clone()).unwrap();
        let by_day = report.by_day(window, zone).unwrap();
        assert_eq!(days.len(), by_day.len());
        for (quarters, day) in days.iter().zip(&by_day) {
            assert_eq!(&quarters.day, day);
            assert_eq!(quarters.quarters.len(), QUARTERS_PER_DAY);
            assert_eq!(
                quarters.quarters.iter().map(|q| q.agent_ms).sum::<u64>(),
                day.active_ms
            );
        }
        assert_eq!(
            days.iter().map(|d| d.day.active_ms).sum::<u64>(),
            report.active_ms
        );
        days
    }

    #[test]
    fn quarters_add_up_to_the_day_and_split_spans_at_quarter_hours() {
        let window = Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-03T00:00:00Z")).unwrap();
        // 09:10–09:40 crosses three quarters; 23:50–00:20 crosses midnight.
        let r = report(&[
            (ms("2026-09-01T09:10:00Z"), ms("2026-09-01T09:40:00Z")),
            (ms("2026-09-01T23:50:00Z"), ms("2026-09-02T00:20:00Z")),
        ]);
        let days = quarters(&r, window, TimeZone::UTC);
        let first = &days[0].quarters;
        assert_eq!(first[36].agent_ms, 5 * M as u64);
        assert_eq!(first[37].agent_ms, 15 * M as u64);
        assert_eq!(first[38].agent_ms, 10 * M as u64);
        assert_eq!(first[95].agent_ms, 10 * M as u64);
        assert_eq!(days[1].quarters[0].agent_ms, 15 * M as u64);
        assert_eq!(days[1].quarters[1].agent_ms, 5 * M as u64);
        assert_eq!(days[0].day.active_ms, 40 * M as u64);
        assert_eq!(days[1].day.active_ms, 20 * M as u64);
        assert!(first.iter().all(|q| q.elapsed_ms == 15 * M as u64));
        // The busiest is the first full quarter.
        assert_eq!(days[0].busiest, Some(37));
        assert_eq!(
            (first[37].agent_ms, first[37].elapsed_ms),
            (15 * M as u64, 15 * M as u64)
        );
    }

    #[test]
    fn overlapping_agents_raise_the_mean() {
        let window = Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-02T00:00:00Z")).unwrap();
        let at = ms("2026-09-01T14:00:00Z");
        // Three agents across 14:00–14:15, a fourth for 5 minutes of it;
        // five across 15:00–15:15.
        let mut spans = vec![(at, at + 15 * M); 3];
        spans.push((at, at + 5 * M));
        spans.extend(vec![(at + 60 * M, at + 75 * M); 5]);
        let r = report(&spans);
        let day = &quarters(&r, window, TimeZone::UTC)[0];
        let q56 = &day.quarters[56];
        assert_eq!(q56.agent_ms, 50 * M as u64);
        assert_eq!(q56.elapsed_ms, 15 * M as u64);
        assert_eq!(day.quarters[60].agent_ms, 5 * 15 * M as u64);
        assert_eq!(day.busiest, Some(60));
        // Two agents for the whole quarter: twice its real length.
        let r = report(&[(at, at + 15 * M), (at, at + 15 * M)]);
        let day = &quarters(&r, window, TimeZone::UTC)[0];
        assert_eq!(day.quarters[56].agent_ms, 2 * day.quarters[56].elapsed_ms);
    }

    #[test]
    fn busiest_is_the_first_of_equal_quarters_and_none_on_an_empty_day() {
        let window = Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-03T00:00:00Z")).unwrap();
        let r = report(&[
            (ms("2026-09-01T08:00:00Z"), ms("2026-09-01T08:15:00Z")),
            (ms("2026-09-01T20:00:00Z"), ms("2026-09-01T20:15:00Z")),
        ]);
        let days = quarters(&r, window, TimeZone::UTC);
        assert_eq!(days[0].busiest, Some(32));
        assert_eq!(days[1].busiest, None);
        assert_eq!(days[1].day.active_ms, 0);
        assert!(days[1].quarters.iter().all(|q| q.agent_ms == 0));
    }

    #[test]
    fn a_fall_back_day_folds_the_repeated_hour_into_the_same_quarters() {
        // Los Angeles, 2026-11-01: 01:00–02:00 happens twice (25-hour day).
        let zone = TimeZone::get("America/Los_Angeles").unwrap();
        let window = Window::new(ms("2026-11-01T07:00:00Z"), ms("2026-11-02T08:00:00Z")).unwrap();
        // One agent through both passes of 01:00–02:00 (08:00–10:00 UTC).
        let r = report(&[(ms("2026-11-01T08:00:00Z"), ms("2026-11-01T10:00:00Z"))]);
        let day = &quarters(&r, window, zone)[0];
        assert_eq!(day.day.active_ms, 120 * M as u64);
        for quarter in &day.quarters[4..8] {
            assert_eq!(quarter.agent_ms, 30 * M as u64);
            assert_eq!(quarter.elapsed_ms, 30 * M as u64);
            // Agent time equals real time: one agent on average, not two.
            assert_eq!(quarter.agent_ms, quarter.elapsed_ms);
        }
        assert_eq!(day.quarters[3].elapsed_ms, 15 * M as u64);
        assert_eq!(day.quarters[8].elapsed_ms, 15 * M as u64);
        assert_eq!(
            day.quarters.iter().map(|q| q.elapsed_ms).sum::<u64>(),
            25 * 60 * M as u64
        );
        assert_eq!(day.busiest, Some(4));
    }

    #[test]
    fn a_spring_forward_day_leaves_the_skipped_hour_empty() {
        // Los Angeles, 2026-03-08: 02:00–03:00 does not happen (23-hour day).
        let zone = TimeZone::get("America/Los_Angeles").unwrap();
        let window = Window::new(ms("2026-03-08T08:00:00Z"), ms("2026-03-09T07:00:00Z")).unwrap();
        // 01:30 PST to 03:30 PDT is one real hour.
        let r = report(&[(ms("2026-03-08T09:30:00Z"), ms("2026-03-08T10:30:00Z"))]);
        let day = &quarters(&r, window, zone)[0];
        assert_eq!(day.day.active_ms, 60 * M as u64);
        for quarter in &day.quarters[8..12] {
            assert_eq!(quarter.elapsed_ms, 0);
            assert_eq!(quarter.agent_ms, 0);
        }
        for index in [6, 7, 12, 13] {
            assert_eq!(day.quarters[index].agent_ms, 15 * M as u64);
            assert_eq!(day.quarters[index].elapsed_ms, 15 * M as u64);
        }
        assert_eq!(
            day.quarters.iter().map(|q| q.elapsed_ms).sum::<u64>(),
            23 * 60 * M as u64
        );
    }

    #[test]
    fn a_span_past_the_window_end_stays_on_the_last_day() {
        // As `by_day` does for a leap-second projection past the end.
        let end = ms("2017-01-01T00:00:00Z");
        let window = Window::new(end - 86_400_000, end).unwrap();
        let r = report(&[(end + 100, end + 900)]);
        let day = &quarters(&r, window, TimeZone::UTC)[0];
        assert_eq!(day.quarters[95].agent_ms, 800);
    }
}
