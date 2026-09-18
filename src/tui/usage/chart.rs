use super::*;
use chrono::{NaiveDate, Timelike};
use ratatui::widgets::{Bar, BarChart, BarGroup};

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

impl App {
    pub(super) fn draw_usage_chart(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        page: &UsagePage,
    ) {
        page.table_area.set(Rect::default());
        page.limit.set(0);
        let chart_area = Rect {
            y: area.y.saturating_add(1),
            height: area.height.saturating_sub(1),
            ..area
        };
        if chart_area.height < 6 {
            frame.render_widget(
                Paragraph::new("Enlarge terminal to view chart").style(Style::default().fg(MUTED)),
                chart_area,
            );
            return;
        }
        let series = series(&self.usage.snapshot, page);
        let capacity = usize::from(chart_area.width / 2).max(1);
        let group = series.bins.len().div_ceil(capacity).max(1);
        let bins = coarsen(&series.bins, group);
        let width = ((usize::from(chart_area.width) + 1) / bins.len().max(1))
            .saturating_sub(1)
            .clamp(1, 8) as u16;
        let step = series.step * group;
        let missing = page.chart_tokens && bins.iter().any(|bin| bin.unknown > 0);
        let values: Vec<Option<u64>> = bins
            .iter()
            .map(|bin| {
                if page.chart_tokens && bin.unknown > 0 {
                    None
                } else {
                    Some(if page.chart_tokens {
                        (bin.input + bin.output).max(0) as u64
                    } else {
                        bin.calls.max(0) as u64
                    })
                }
            })
            .collect();
        let max = values.iter().flatten().copied().max().unwrap_or(0);
        let bars = values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let label = if width >= 5 {
                    if series.hourly {
                        format!("{:02}:00", i * step)
                    } else {
                        (series.start + chrono::Duration::days((i * step) as i64))
                            .format("%m/%d")
                            .to_string()
                    }
                } else {
                    String::new()
                };
                Bar::default()
                    .value(value.unwrap_or(0))
                    .label(Line::styled(label, Style::default().fg(MUTED)))
                    .text_value(value.map(|v| short_value(v, width)).unwrap_or_default())
            })
            .collect::<Vec<_>>();
        let interval = if step == 1 {
            if series.hourly {
                "hour".into()
            } else {
                "day".into()
            }
        } else {
            format!("{step} {}", if series.hourly { "hours" } else { "days" })
        };
        let title = format!(
            " {} / {interval} · max {}{} ",
            if page.chart_tokens { "Tokens" } else { "Calls" },
            max,
            if missing { " · ? unknown" } else { "" }
        );
        // Unknown buckets use '?' on the value baseline; zero buckets use '0'.
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(MUTED))
            .title(title);
        let plot = block.inner(chart_area);
        frame.render_widget(block, chart_area);
        let bars_area = Rect {
            height: plot.height.saturating_sub(u16::from(width < 5)),
            ..plot
        };
        let chart = BarChart::default()
            .data(BarGroup::default().bars(&bars))
            .bar_width(width)
            .bar_gap(1)
            .max(max.max(1))
            .bar_style(Style::default().fg(ROUTE))
            .value_style(Style::default().fg(Color::Black).bg(ROUTE));
        frame.render_widget(chart, bars_area);
        for (i, value) in values.iter().enumerate() {
            if value.is_none() || *value == Some(0) {
                let x = plot.x + i as u16 * (width + 1);
                if x < plot.right() {
                    frame.render_widget(
                        Paragraph::new(if value.is_none() { "?" } else { "0" })
                            .alignment(Alignment::Center)
                            .style(Style::default().fg(if value.is_none() {
                                WARNING
                            } else {
                                MUTED
                            })),
                        Rect::new(
                            x,
                            bars_area.bottom().saturating_sub(2),
                            width.min(plot.right() - x),
                            1,
                        ),
                    );
                }
            }
        }
        let axis = Rect::new(
            plot.x,
            plot.bottom().saturating_sub(1),
            ((bins.len() as u16) * (width + 1))
                .saturating_sub(1)
                .min(plot.width),
            1,
        );
        if width < 5 {
            frame.render_widget(
                Paragraph::new(series.first).style(Style::default().fg(MUTED)),
                axis,
            );
            if bins.len() > 1 {
                frame.render_widget(
                    Paragraph::new(series.last)
                        .alignment(Alignment::Right)
                        .style(Style::default().fg(MUTED)),
                    axis,
                );
            }
        }
    }
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
