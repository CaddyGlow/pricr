use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::symbols;
use ratatui::text::Line;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Widget};

use crate::provider::PriceHistory;

const MIN_WIDTH: u16 = 48;
const MIN_HEIGHT: u16 = 12;

/// Render a static terminal chart for a coin price history series.
pub fn render_history_chart(history: &PriceHistory, width: u16, height: u16) -> String {
    if history.points.is_empty() {
        return String::new();
    }

    let area = Rect::new(0, 0, width.max(MIN_WIDTH), height.max(MIN_HEIGHT));
    let points: Vec<(f64, f64)> = history
        .points
        .iter()
        .enumerate()
        .map(|(idx, p)| (idx as f64, p.price))
        .collect();

    let x_max = points.len().saturating_sub(1) as f64;
    let (y_min, y_max) = y_bounds(&points);

    let first_label = history
        .points
        .first()
        .map(|p| p.timestamp.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let last_label = history
        .points
        .last()
        .map(|p| p.timestamp.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let dataset = Dataset::default()
        .name(history.symbol.as_str())
        .graph_type(GraphType::Line)
        .marker(symbols::Marker::Dot)
        .data(&points);

    let chart = Chart::new(vec![dataset])
        .block(
            Block::default()
                .title(format!("{} Price History", history.symbol))
                .borders(Borders::ALL),
        )
        .x_axis(
            Axis::default()
                .title(Line::from("Time"))
                .bounds([0.0, x_max.max(1.0)])
                .labels(vec![Line::from(first_label), Line::from(last_label)]),
        )
        .y_axis(
            Axis::default()
                .title(Line::from(history.currency.clone()))
                .bounds([y_min, y_max])
                .labels(vec![
                    Line::from(format_price_label(y_min)),
                    Line::from(format_price_label(y_max)),
                ]),
        );

    let mut buffer = Buffer::empty(area);
    chart.render(area, &mut buffer);
    buffer_to_string(&buffer, area)
}

fn y_bounds(points: &[(f64, f64)]) -> (f64, f64) {
    let min = points.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
    let max = points
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);

    let span = max - min;
    if span <= f64::EPSILON {
        let padding = if max.abs() <= 1.0 {
            1.0
        } else {
            (max.abs() * 0.01).max(1.0)
        };
        (min - padding, max + padding)
    } else {
        let padding = span * 0.08;
        (min - padding, max + padding)
    }
}

fn format_price_label(value: f64) -> String {
    if value.abs() >= 1_000.0 {
        format!("{value:.0}")
    } else if value.abs() >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn buffer_to_string(buffer: &Buffer, area: Rect) -> String {
    let mut lines = Vec::with_capacity(area.height as usize);
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            line.push_str(buffer[(x, y)].symbol());
        }

        while line.ends_with(' ') {
            line.pop();
        }

        lines.push(line);
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{PriceHistory, PricePoint};

    #[test]
    fn render_history_chart_outputs_box() {
        let history = PriceHistory {
            symbol: "BTC".to_string(),
            name: "Bitcoin".to_string(),
            currency: "USD".to_string(),
            provider: "CoinGecko".to_string(),
            points: vec![
                PricePoint {
                    timestamp: chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000, 0)
                        .expect("valid timestamp"),
                    price: 40000.0,
                },
                PricePoint {
                    timestamp: chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_086_400, 0)
                        .expect("valid timestamp"),
                    price: 42000.0,
                },
            ],
        };

        let rendered = render_history_chart(&history, 60, 14);
        assert!(!rendered.is_empty());
        assert!(rendered.lines().count() >= 10);
        assert!(rendered.contains("BTC Price History"));
    }
}
