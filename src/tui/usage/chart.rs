use super::*;
use chrono::{NaiveDate, Timelike};

struct Series {
    bins: Vec<Totals>,
    first: String,
    last: String,
    step: usize,
    hourly: bool,
    start: NaiveDate,
}

fn series(snapshot: &Snapshot, page: &UsagePage) -> Series {
    let hourly = page.range == 0;
    let today = snapshot.today();
    let end =
        NaiveDate::parse_from_str(if page.range == 3 { &today } else { &page.day }, "%Y-%m-%d")
            .unwrap();
    let selected: Vec<_> = snapshot
        .rows
        .iter()
        .filter(|r| {
            r.kind == "generation"
                && page.includes(&r.day)
                && page.client().is_none_or(|c| c == r.client)
                && page.provider.as_deref().is_none_or(|p| p == r.provider)
        })
        .collect();
    let start = page
        .start_day()
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .or_else(|| {
            selected
                .iter()
                .filter_map(|r| NaiveDate::parse_from_str(&r.day, "%Y-%m-%d").ok())
                .min()
        })
        .unwrap_or(end);
    let hours = if page.day == snapshot.today() {
        let zone = chrono::FixedOffset::east_opt(snapshot.offset).unwrap();
        chrono::Utc::now().with_timezone(&zone).hour() as usize + 1
    } else {
        24
    };
    let count = if hourly {
        hours
    } else {
        (end - start).num_days().max(0) as usize + 1
    };
    let step = count.div_ceil(256).max(1);
    let mut bins = vec![Totals::default(); count.div_ceil(step)];
    for row in selected {
        let index = if hourly {
            row.hour as usize
        } else {
            let Ok(day) = NaiveDate::parse_from_str(&row.day, "%Y-%m-%d") else {
                continue;
            };
            let days = (day - start).num_days();
            if days < 0 {
                continue;
            }
            days as usize
        };
        if index < count {
            bins[index / step].add(&row.totals);
        }
    }
    Series {
        bins,
        first: if hourly {
            "00:00".into()
        } else {
            start.format("%m/%d").to_string()
        },
        last: if hourly {
            format!("{:02}:00", hours - 1)
        } else {
            end.format("%m/%d").to_string()
        },
        step,
        hourly,
        start,
    }
}

fn coarsen(bins: &[Totals], group: usize) -> Vec<Totals> {
    bins.chunks(group.max(1))
        .map(|chunk| {
            let mut total = Totals::default();
            for bin in chunk {
                total.add(bin);
            }
            total
        })
        .collect()
}

fn short_value(value: u64, width: u16) -> String {
    let exact = value.to_string();
    if exact.len() <= width as usize {
        return exact;
    }
    for (unit, divisor) in [("K", 1_000u64), ("M", 1_000_000), ("B", 1_000_000_000)] {
        if value >= divisor {
            let compact = format!("{}{unit}", value / divisor);
            if compact.len() <= width as usize {
                return compact;
            }
        }
    }
    String::new()
}

pub(super) fn lines(
    snapshot: &Snapshot,
    page: &UsagePage,
    tokens: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let series = series(snapshot, page);
    let group = series
        .bins
        .len()
        .div_ceil(usize::from(width / 2).max(1))
        .max(1);
    let bins = coarsen(&series.bins, group);
    let values: Vec<Option<u64>> = bins
        .iter()
        .map(|bin| {
            if tokens && bin.unknown > 0 {
                None
            } else {
                Some(if tokens {
                    bin.input.saturating_add(bin.output).max(0) as u64
                } else {
                    bin.calls.max(0) as u64
                })
            }
        })
        .collect();
    let peak = values.iter().flatten().copied().max().unwrap_or(0);
    let step = series.step * group;
    let color = if tokens { ROUTE } else { ENABLED };
    let mut out = vec![Line::from(vec![
        Span::styled(
            if tokens { "TOKENS" } else { "CALLS" },
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " / {} {} · peak {}",
                step,
                if series.hourly { "hour(s)" } else { "day(s)" },
                short_value(peak, 10)
            ),
            Style::default().fg(FIELD_LABEL),
        ),
    ])];
    let cell_width = (usize::from(width) / values.len().max(1)).max(1);
    for row in (0..6).rev() {
        let mut spans = vec![];
        for value in &values {
            let units = value.map_or(0, |v| {
                if peak == 0 {
                    0
                } else {
                    ((v as f64 / peak as f64) * 48.0).round() as usize
                }
            });
            let fill = units.saturating_sub(row * 8).min(8);
            let symbol = if value.is_none() && row == 0 {
                "?"
            } else if *value == Some(0) && row == 0 {
                "0"
            } else {
                [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][fill]
            };
            let bar_width = cell_width.saturating_sub(1).max(1);
            let paint = if row == 0 && (value.is_none() || *value == Some(0)) {
                let left = bar_width / 2;
                format!(
                    "{}{}{}",
                    " ".repeat(left),
                    symbol,
                    " ".repeat(bar_width - left - 1)
                )
            } else {
                symbol.repeat(bar_width)
            };
            spans.push(Span::styled(
                paint,
                Style::default().fg(if value.is_none() {
                    WARNING
                } else if fill == 0 {
                    MUTED
                } else {
                    color
                }),
            ));
            if cell_width > 1 {
                spans.push(Span::raw(" "));
            }
        }
        out.push(Line::from(spans));
    }
    let middle = if series.hourly {
        format!("{:02}:00", values.len() / 2 * step)
    } else {
        (series.start + chrono::Duration::days((values.len() / 2 * step) as i64))
            .format("%m/%d")
            .to_string()
    };
    let occupied = series.first.len() + middle.len() + series.last.len();
    let spaces = usize::from(width).saturating_sub(occupied);
    out.push(Line::styled(
        format!(
            "{}{}{}{}{}",
            series.first,
            " ".repeat(spaces / 2),
            middle,
            " ".repeat(spaces - spaces / 2),
            series.last
        ),
        Style::default().fg(MUTED),
    ));
    out.push(Line::styled(
        if tokens {
            "0 known zero · ? usage unknown"
        } else {
            "0 no calls · gateway requests"
        },
        Style::default().fg(MUTED),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bar_groups_preserve_totals_and_missing_usage() {
        let bins = vec![
            Totals {
                calls: 2,
                input: 10,
                output: 2,
                ..Default::default()
            },
            Totals {
                calls: 3,
                unknown: 1,
                ..Default::default()
            },
            Totals {
                calls: 4,
                ..Default::default()
            },
        ];
        let grouped = coarsen(&bins, 2);
        assert_eq!(grouped.len(), 2);
        assert_eq!(
            (grouped[0].calls, grouped[0].input, grouped[0].unknown),
            (5, 10, 1)
        );
        assert_eq!(grouped[1].calls, 4);
        assert_eq!(short_value(10000, 3), "10K");
    }
    #[test]
    fn ranges_and_chart_buckets_match_and_fill_quiet_days() {
        let (_temp, mut app) = crate::tui::tests::persisted_app();
        app.open_usage();
        let mut page = app.usage.page.take().unwrap();
        page.day = "2026-09-18".into();
        page.follow_today = false;
        let mut snapshot = Snapshot::default();
        for (day, hour, calls) in [
            ("2026-09-18", 3, 2),
            ("2026-09-18", 8, 3),
            ("2026-09-12", 7, 5),
            ("2026-09-11", 2, 7),
            ("2026-08-20", 3, 11),
            ("2026-08-19", 1, 13),
        ] {
            snapshot.rows.push(crate::usage::Row {
                day: day.into(),
                hour,
                client: "Claude".into(),
                provider: "p".into(),
                name: "P".into(),
                model: "m".into(),
                kind: "generation".into(),
                totals: Totals {
                    calls,
                    ..Default::default()
                },
            });
        }
        assert_eq!(
            page.range_total(&snapshot, None, None, "generation").calls,
            5
        );
        page.day = "2026-09-11".into();
        let hourly = series(&snapshot, &page);
        assert_eq!(hourly.bins.len(), 24);
        assert_eq!(hourly.bins[2].calls, 7);
        assert_eq!(hourly.bins[1].calls, 0);
        page.day = "2026-09-18".into();
        page.range = 1;
        assert_eq!(
            page.range_total(&snapshot, None, None, "generation").calls,
            10
        );
        let s = series(&snapshot, &page);
        assert_eq!(s.bins.len(), 7);
        assert_eq!(s.bins[0].calls, 5);
        assert_eq!(s.bins[1].calls, 0);
        assert_eq!(s.bins[6].calls, 5);
        page.range = 2;
        assert_eq!(
            page.range_total(&snapshot, None, None, "generation").calls,
            28
        );
        assert_eq!(series(&snapshot, &page).bins.len(), 30);
        page.range = 3;
        assert_eq!(
            page.range_total(&snapshot, None, None, "generation").calls,
            41
        );
        page.provider = Some("other".into());
        assert_eq!(
            page.range_total(&snapshot, None, Some("other"), "generation")
                .calls,
            0
        );
        assert!(series(&snapshot, &page).bins.iter().all(|b| b.calls == 0));
    }
}
