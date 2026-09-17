//! Zero-dependency deterministic SVG renderer for the backend-neutral plot
//! specs produced by `DataFrame::plot` / `Series::plot` / `hist` / `boxplot`
//! and the GroupBy plotting hooks.
//!
//! Design constraints (br-frankenpandas-rc-plot-renderer-zf9bf):
//! - **Pure Rust, no new dependencies, no `unsafe`, Tokio-free.** SVG is text;
//!   an in-house writer beats pulling a font/rasterizer stack into the
//!   workspace. PNG output needs a rasterizer dependency and is a deliberate
//!   follow-up, not part of this module.
//! - **Deterministic**: no clock, no RNG, fixed palette and geometry — the same
//!   spec renders byte-identical SVG forever (asserted by a test).
//! - **Fail closed, zero panics**: non-numeric series, all-missing series, and
//!   negative pie values return `Err(FrameError::...)` instead of plotting
//!   nonsense or panicking; `NaN`/`Null` values become gaps (line/area/scatter)
//!   or are skipped (histogram/box/bar).
//!
//! Style floor only: axes, ticks, gridlines, series colors, titles, legend,
//! XML-escaped labels. This is not matplotlib parity by design.

use crate::{BoxPlotSpec, FrameError, HistogramSpec, PlotKind, PlotSeriesSpec, PlotSpec, Scalar};

// ── Geometry / palette ─────────────────────────────────────────────────────

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 400.0;
const MARGIN_LEFT: f64 = 56.0;
const MARGIN_RIGHT: f64 = 16.0;
const MARGIN_TOP: f64 = 30.0;
const MARGIN_BOTTOM: f64 = 42.0;
const PLOT_W: f64 = WIDTH - MARGIN_LEFT - MARGIN_RIGHT;
const PLOT_H: f64 = HEIGHT - MARGIN_TOP - MARGIN_BOTTOM;
const PALETTE: [&str; 7] = [
    "#4e79a7", "#f28e2b", "#e15759", "#76b7b2", "#59a14f", "#edc948", "#b07aa1",
];
const MAX_X_TICKS: usize = 8;

fn palette(i: usize) -> &'static str {
    PALETTE[i % PALETTE.len()]
}

/// XML-escape text destined for element content or attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Numeric view of a series: `None` = missing (`NaN` or `Null`), `Err` = the
/// series is not plottable at all (non-numeric dtype). Matches pandas, where
/// plotting an object column raises instead of inventing numbers.
fn numeric_view(series: &PlotSeriesSpec) -> Result<Vec<Option<f64>>, FrameError> {
    series
        .values
        .iter()
        .map(|value| match value {
            Scalar::Float64(v) => Ok(if v.is_finite() { Some(*v) } else { None }),
            Scalar::Int64(v) => Ok(Some(*v as f64)),
            Scalar::Bool(b) => Ok(Some(if *b { 1.0 } else { 0.0 })),
            Scalar::Null(_) => Ok(None),
            other => Err(FrameError::CompatibilityRejected(format!(
                "plot requires numeric values: series '{}' carries {:?} ({other:?})",
                series.name, series.dtype
            ))),
        })
        .collect()
}

/// A finite min/max pair across every series; constant data is padded so the
/// band keeps height instead of dividing by zero. All-missing data is an error.
#[derive(Debug, Clone, Copy)]
struct Scale {
    min: f64,
    max: f64,
}

fn data_scale(series: &[Vec<Option<f64>>]) -> Result<Scale, FrameError> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for values in series {
        for v in values.iter().flatten() {
            if v.is_finite() {
                min = min.min(*v);
                max = max.max(*v);
            }
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return Err(FrameError::CompatibilityRejected(
            "no plottable (non-missing) values in this plot".to_owned(),
        ));
    }
    if (max - min).abs() < f64::EPSILON {
        min -= 1.0;
        max += 1.0;
    }
    Ok(Scale { min, max })
}

fn fmt_num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.4}")
    }
}

fn svg_open(title: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" font-family=\"monospace\" font-size=\"11\"><rect width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"white\"/><text x=\"{MARGIN_LEFT}\" y=\"18\" fill=\"#333\">{}</text>",
        esc(title)
    )
}

fn svg_axes(scale: &Scale, x_labels: &[String]) -> String {
    let mut out = String::new();
    let right = MARGIN_LEFT + PLOT_W;
    let bottom = MARGIN_TOP + PLOT_H;
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let value = scale.max - frac * (scale.max - scale.min);
        let y = MARGIN_TOP + frac * PLOT_H;
        out.push_str(&format!(
            "<line x1=\"{MARGIN_LEFT}\" y1=\"{y:.2}\" x2=\"{right}\" y2=\"{y:.2}\" stroke=\"#e5e5e5\"/><text x=\"{}\" y=\"{:.2}\" text-anchor=\"end\" fill=\"#666\">{}</text>",
            MARGIN_LEFT - 6.0,
            y + 4.0,
            esc(&fmt_num(value))
        ));
    }
    out.push_str(&format!(
        "<line x1=\"{MARGIN_LEFT}\" y1=\"{MARGIN_TOP}\" x2=\"{MARGIN_LEFT}\" y2=\"{bottom}\" stroke=\"#333\"/><line x1=\"{MARGIN_LEFT}\" y1=\"{bottom}\" x2=\"{right}\" y2=\"{bottom}\" stroke=\"#333\"/>"
    ));
    let n = x_labels.len();
    if n > 0 {
        let step = n.div_ceil(MAX_X_TICKS);
        for (i, label) in x_labels.iter().enumerate() {
            if i % step != 0 {
                continue;
            }
            let x = MARGIN_LEFT + (i as f64 + 0.5) * PLOT_W / n as f64;
            out.push_str(&format!(
                "<text x=\"{x:.2}\" y=\"{:.2}\" text-anchor=\"middle\" fill=\"#666\">{}</text>",
                bottom + 14.0,
                esc(label)
            ));
        }
    }
    out
}

fn svg_horizontal_axes(scale: &Scale, y_labels: &[String]) -> String {
    let mut out = String::new();
    let right = MARGIN_LEFT + PLOT_W;
    let bottom = MARGIN_TOP + PLOT_H;
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let value = scale.min + frac * (scale.max - scale.min);
        let x = MARGIN_LEFT + frac * PLOT_W;
        out.push_str(&format!(
            "<line x1=\"{x:.2}\" y1=\"{MARGIN_TOP}\" x2=\"{x:.2}\" y2=\"{bottom}\" stroke=\"#e5e5e5\"/><text x=\"{x:.2}\" y=\"{:.2}\" text-anchor=\"middle\" fill=\"#666\">{}</text>",
            bottom + 14.0,
            esc(&fmt_num(value))
        ));
    }
    out.push_str(&format!(
        "<line x1=\"{MARGIN_LEFT}\" y1=\"{MARGIN_TOP}\" x2=\"{MARGIN_LEFT}\" y2=\"{bottom}\" stroke=\"#333\"/><line x1=\"{MARGIN_LEFT}\" y1=\"{bottom}\" x2=\"{right}\" y2=\"{bottom}\" stroke=\"#333\"/>"
    ));
    let n = y_labels.len();
    if n > 0 {
        let step = n.div_ceil(MAX_X_TICKS);
        let slot = PLOT_H / n as f64;
        for (i, label) in y_labels.iter().enumerate() {
            if i % step != 0 {
                continue;
            }
            let y = MARGIN_TOP + (i as f64 + 0.5) * slot;
            let truncated = if label.len() > 8 {
                format!("{}…", &label[..7])
            } else {
                label.clone()
            };
            out.push_str(&format!(
                "<text x=\"{}\" y=\"{y:.2}\" text-anchor=\"end\" dominant-baseline=\"middle\" fill=\"#666\">{}</text>",
                MARGIN_LEFT - 6.0,
                esc(&truncated)
            ));
        }
    }
    out
}

fn svg_numeric_2d_axes(x_scale: &Scale, y_scale: &Scale) -> String {
    let mut out = String::new();
    let right = MARGIN_LEFT + PLOT_W;
    let bottom = MARGIN_TOP + PLOT_H;
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let value = y_scale.max - frac * (y_scale.max - y_scale.min);
        let y = MARGIN_TOP + frac * PLOT_H;
        out.push_str(&format!(
            "<line x1=\"{MARGIN_LEFT}\" y1=\"{y:.2}\" x2=\"{right}\" y2=\"{y:.2}\" stroke=\"#e5e5e5\"/><text x=\"{}\" y=\"{:.2}\" text-anchor=\"end\" fill=\"#666\">{}</text>",
            MARGIN_LEFT - 6.0,
            y + 4.0,
            esc(&fmt_num(value))
        ));
    }
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let value = x_scale.min + frac * (x_scale.max - x_scale.min);
        let x = MARGIN_LEFT + frac * PLOT_W;
        out.push_str(&format!(
            "<line x1=\"{x:.2}\" y1=\"{MARGIN_TOP}\" x2=\"{x:.2}\" y2=\"{bottom}\" stroke=\"#e5e5e5\"/><text x=\"{x:.2}\" y=\"{:.2}\" text-anchor=\"middle\" fill=\"#666\">{}</text>",
            bottom + 14.0,
            esc(&fmt_num(value))
        ));
    }
    out.push_str(&format!(
        "<line x1=\"{MARGIN_LEFT}\" y1=\"{MARGIN_TOP}\" x2=\"{MARGIN_LEFT}\" y2=\"{bottom}\" stroke=\"#333\"/><line x1=\"{MARGIN_LEFT}\" y1=\"{bottom}\" x2=\"{right}\" y2=\"{bottom}\" stroke=\"#333\"/>"
    ));
    out
}

fn legend(entries: &[(String, &str)]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let y = HEIGHT - 10.0;
    let max_display = 6;
    let (display_entries, overflow) = if entries.len() > max_display {
        (
            &entries[..max_display - 1],
            Some(entries.len() - (max_display - 1)),
        )
    } else {
        (entries, None)
    };
    let total_items = display_entries.len() + if overflow.is_some() { 1 } else { 0 };
    let spacing = (PLOT_W / total_items.max(1) as f64).min(140.0);
    for (i, (name, color)) in display_entries.iter().enumerate() {
        let x = MARGIN_LEFT + (i as f64 * spacing);
        let max_chars = ((spacing - 20.0) / 7.0).floor() as usize;
        let display_name = if name.chars().count() > max_chars && max_chars >= 4 {
            let truncated: String = name.chars().take(max_chars - 1).collect();
            format!("{truncated}…")
        } else {
            name.clone()
        };
        out.push_str(&format!(
            "<rect x=\"{x:.2}\" y=\"{:.2}\" width=\"10\" height=\"10\" fill=\"{color}\"/><text x=\"{:.2}\" y=\"{y:.2}\" fill=\"#333\">{}</text>",
            y - 9.0,
            x + 14.0,
            esc(&display_name)
        ));
    }
    if let Some(count) = overflow {
        let x = MARGIN_LEFT + (display_entries.len() as f64 * spacing);
        out.push_str(&format!(
            "<text x=\"{x:.2}\" y=\"{y:.2}\" fill=\"#666\">+{count} more</text>"
        ));
    }
    out
}

// ── PlotSpec rendering (line / area / scatter / bar / pie) ─────────────────

fn render_plot(spec: &PlotSpec) -> Result<String, FrameError> {
    if spec.kind == PlotKind::Box {
        return boxplot_body(&spec.series, &spec.method);
    }
    if spec.kind == PlotKind::Histogram {
        return histogram_body(&spec.series, 10, &spec.method);
    }

    let views: Result<Vec<Vec<Option<f64>>>, FrameError> =
        spec.series.iter().map(numeric_view).collect();
    let views = views?;

    let is_xy_scatter = spec.kind == PlotKind::Scatter
        && spec.series.len() >= 2
        && spec.series.len().is_multiple_of(2)
        && (spec.method.contains("(x=")
            || spec.method.contains("scatter_columns")
            || spec.method.contains(".lag(")
            || spec.method.contains("radviz"));
    let is_xy_hexbin = spec.kind == PlotKind::Hexbin
        && spec.series.len() >= 2
        && spec.series.len().is_multiple_of(2)
        && (spec.method.contains("(x=") || spec.method.contains("hexbin_columns"));

    let (scale, xy_scales) = if (is_xy_scatter || is_xy_hexbin) && views.len() >= 2 {
        let x_views: Vec<_> = (0..views.len())
            .step_by(2)
            .map(|i| views[i].clone())
            .collect();
        let y_views: Vec<_> = (1..views.len())
            .step_by(2)
            .map(|i| views[i].clone())
            .collect();
        let (x_sc, y_sc) = if spec.method.contains("radviz") {
            (
                Scale {
                    min: -1.15,
                    max: 1.15,
                },
                Scale {
                    min: -1.15,
                    max: 1.15,
                },
            )
        } else {
            (data_scale(&x_views)?, data_scale(&y_views)?)
        };
        (y_sc, Some((x_sc, y_sc)))
    } else {
        let mut sc = data_scale(&views)?;
        if spec.kind == PlotKind::Bar || spec.kind == PlotKind::Barh {
            sc.min = sc.min.min(0.0);
            sc.max = sc.max.max(0.0);
            if (sc.max - sc.min).abs() < f64::EPSILON {
                sc.max += 1.0;
            }
        }
        (sc, None)
    };

    let n = spec
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0);
    let mut x_labels: Vec<String> = spec
        .series
        .first()
        .map(|first| {
            first
                .index
                .iter()
                .map(|label| crate::scalar_plot_label(&crate::index_label_to_scalar(label)))
                .collect()
        })
        .unwrap_or_default();
    if x_labels.len() < n {
        x_labels = (0..n).map(|i| i.to_string()).collect();
    }

    let y = |v: f64| MARGIN_TOP + (scale.max - v) / (scale.max - scale.min) * PLOT_H;
    let x = |i: usize| MARGIN_LEFT + (i as f64 + 0.5) * PLOT_W / n.max(1) as f64;

    let mut body = String::new();
    match spec.kind {
        PlotKind::Line | PlotKind::Area => {
            let is_class_grouped = spec.kind == PlotKind::Line
                && (spec.method.contains("parallel_coordinates")
                    || spec.method.contains("andrews_curves"));
            let mut unique_classes: Vec<String> = Vec::new();
            if is_class_grouped {
                for s in &spec.series {
                    let class_name = s
                        .group_key
                        .as_ref()
                        .and_then(|gk| gk.first())
                        .map(crate::scalar_plot_label)
                        .unwrap_or_else(|| s.name.clone());
                    if !unique_classes.contains(&class_name) {
                        unique_classes.push(class_name);
                    }
                }
            }

            if spec.method.contains("parallel_coordinates") {
                let bottom = MARGIN_TOP + PLOT_H;
                for i in 0..n {
                    let xi = x(i);
                    body.push_str(&format!(
                        "<line x1=\"{xi:.2}\" y1=\"{MARGIN_TOP}\" x2=\"{xi:.2}\" y2=\"{bottom}\" stroke=\"#ccc\" stroke-width=\"1\" stroke-dasharray=\"3,3\"/>"
                    ));
                }
            }

            // Contiguous finite runs become separate polylines; NaN/Null breaks
            // the line exactly like a masked gap.
            for (si, values) in views.iter().enumerate() {
                let color = if is_class_grouped {
                    let class_name = spec.series[si]
                        .group_key
                        .as_ref()
                        .and_then(|gk| gk.first())
                        .map(crate::scalar_plot_label)
                        .unwrap_or_else(|| spec.series[si].name.clone());
                    let c_idx = unique_classes
                        .iter()
                        .position(|c| c == &class_name)
                        .unwrap_or(si);
                    palette(c_idx)
                } else {
                    palette(si)
                };
                let stroke_attrs = if is_class_grouped {
                    "stroke-width=\"1.5\" stroke-opacity=\"0.6\""
                } else {
                    "stroke-width=\"2\""
                };
                let mut runs: Vec<Vec<(f64, f64)>> = vec![Vec::new()];
                for (i, v) in values.iter().enumerate() {
                    match v.filter(|v| v.is_finite()) {
                        Some(v) => runs.last_mut().expect("seeded run").push((x(i), y(v))),
                        None if !runs.last().expect("seeded run").is_empty() => {
                            runs.push(Vec::new());
                        }
                        None => {}
                    }
                }
                runs.retain(|run| !run.is_empty());
                for run in &runs {
                    let points = run
                        .iter()
                        .map(|(px, py)| format!("{px:.2},{py:.2}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    if spec.kind == PlotKind::Area {
                        let first_x = run[0].0;
                        let last_x = run[run.len() - 1].0;
                        let base = y(scale.min);
                        body.push_str(&format!(
                            "<polygon points=\"{first_x:.2},{base:.2} {points} {last_x:.2},{base:.2}\" fill=\"{color}\" fill-opacity=\"0.25\" stroke=\"none\"/>"
                        ));
                    }
                    body.push_str(&format!(
                        "<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" {stroke_attrs}/>"
                    ));
                }
            }
        }
        PlotKind::Scatter => {
            if let Some((ref x_sc, ref y_sc)) = xy_scales {
                let x_map = |v: f64| MARGIN_LEFT + (v - x_sc.min) / (x_sc.max - x_sc.min) * PLOT_W;
                let y_map = |v: f64| MARGIN_TOP + (y_sc.max - v) / (y_sc.max - y_sc.min) * PLOT_H;
                if spec.method.contains("radviz") {
                    let cx = x_map(0.0);
                    let cy = y_map(0.0);
                    let rx = (x_map(1.0) - cx).abs();
                    let ry = (y_map(1.0) - cy).abs();
                    body.push_str(&format!(
                        "<ellipse cx=\"{cx:.2}\" cy=\"{cy:.2}\" rx=\"{rx:.2}\" ry=\"{ry:.2}\" fill=\"none\" stroke=\"#bbb\" stroke-width=\"1.5\"/>"
                    ));
                }
                let num_groups = views.len() / 2;
                for g in 0..num_groups {
                    let color = palette(g);
                    let vx_view = &views[2 * g];
                    let vy_view = &views[2 * g + 1];
                    let len = vx_view.len().min(vy_view.len());
                    for i in 0..len {
                        if let (Some(vx), Some(vy)) = (vx_view[i], vy_view[i])
                            && vx.is_finite()
                            && vy.is_finite()
                        {
                            body.push_str(&format!(
                                "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"2.5\" fill=\"{color}\"/>",
                                x_map(vx),
                                y_map(vy)
                            ));
                        }
                    }
                }
            } else {
                for (si, values) in views.iter().enumerate() {
                    let color = palette(si);
                    for (i, v) in values.iter().enumerate() {
                        if let Some(v) = v.filter(|v| v.is_finite()) {
                            body.push_str(&format!(
                                "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"2.5\" fill=\"{color}\"/>",
                                x(i),
                                y(v)
                            ));
                        }
                    }
                }
            }
        }
        PlotKind::Bar => {
            let k = views.len().max(1);
            let slot = PLOT_W / n.max(1) as f64;
            let bar_w = (slot * 0.7 / k as f64).max(0.5);
            let baseline = 0.0f64.clamp(scale.min, scale.max);
            let base = y(baseline);
            for (si, values) in views.iter().enumerate() {
                let color = palette(si);
                let offset = si as f64 * bar_w + slot * 0.15;
                for (i, v) in values.iter().enumerate() {
                    if let Some(v) = v.filter(|v| v.is_finite()) {
                        let top = y(v);
                        let rect_y = top.min(base);
                        let rect_h = (base - top).abs().max(0.5);
                        body.push_str(&format!(
                            "<rect x=\"{:.2}\" y=\"{rect_y:.2}\" width=\"{bar_w:.2}\" height=\"{rect_h:.2}\" fill=\"{color}\"/>",
                            MARGIN_LEFT + i as f64 * slot + offset,
                        ));
                    }
                }
            }
        }
        PlotKind::Pie => {
            let any_positive = views.iter().any(|v| v.iter().flatten().any(|x| *x > 0.0));
            if !any_positive {
                return Err(FrameError::CompatibilityRejected(
                    "pie plot requires at least one positive value".to_owned(),
                ));
            }
            let mut angle: f64 = -std::f64::consts::FRAC_PI_2;
            let cx = MARGIN_LEFT + PLOT_W / 2.0;
            let cy = MARGIN_TOP + PLOT_H / 2.0;
            let r = 140.0;
            let mut slice_idx = 0;
            for (si, values) in views.iter().enumerate() {
                let finite_values: Vec<f64> = values
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|v| v.is_finite())
                    .collect();
                if finite_values.iter().any(|v| *v < 0.0) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "pie requires non-negative values: series '{}' has a negative slice",
                        spec.series[si].name
                    )));
                }
                let total: f64 = finite_values.iter().sum();
                if total <= 0.0 {
                    continue;
                }
                for v in finite_values {
                    if v <= 0.0 {
                        continue;
                    }
                    let color = palette(slice_idx);
                    slice_idx += 1;
                    let frac = v / total;
                    if frac >= 1.0 - 1e-6 {
                        body.push_str(&format!(
                            "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r}\" fill=\"{color}\" stroke=\"white\"/>"
                        ));
                        angle += std::f64::consts::TAU;
                    } else {
                        let next = angle + frac * std::f64::consts::TAU;
                        let large = (next - angle) > std::f64::consts::PI;
                        body.push_str(&format!(
                            "<path d=\"M {cx:.2} {cy:.2} L {:.2} {:.2} A {r} {r} 0 {} 1 {:.2} {:.2} Z\" fill=\"{color}\" stroke=\"white\"/>",
                            cx + r * angle.cos(),
                            cy + r * angle.sin(),
                            i64::from(large),
                            cx + r * next.cos(),
                            cy + r * next.sin(),
                        ));
                        angle = next;
                    }
                }
            }
        }
        PlotKind::Barh => {
            let k = views.len().max(1);
            let slot = PLOT_H / n.max(1) as f64;
            let bar_h = (slot * 0.7 / k as f64).max(0.5);
            let baseline = 0.0f64.clamp(scale.min, scale.max);
            let base_x = MARGIN_LEFT + (baseline - scale.min) / (scale.max - scale.min) * PLOT_W;
            let val_to_x =
                |v: f64| MARGIN_LEFT + (v - scale.min) / (scale.max - scale.min) * PLOT_W;
            for (si, values) in views.iter().enumerate() {
                let color = palette(si);
                let offset = si as f64 * bar_h + slot * 0.15;
                for (i, v) in values.iter().enumerate() {
                    if let Some(v) = v.filter(|v| v.is_finite()) {
                        let vx = val_to_x(v);
                        let rect_x = vx.min(base_x);
                        let rect_w = (vx - base_x).abs().max(0.5);
                        let rect_y = MARGIN_TOP + i as f64 * slot + offset;
                        body.push_str(&format!(
                            "<rect x=\"{rect_x:.2}\" y=\"{rect_y:.2}\" width=\"{rect_w:.2}\" height=\"{bar_h:.2}\" fill=\"{color}\"/>",
                        ));
                    }
                }
            }
        }
        PlotKind::Kde | PlotKind::Density => {
            let mut all_finite: Vec<f64> = Vec::new();
            let mut per_series_finite: Vec<Vec<f64>> = Vec::new();
            for values in &views {
                let finite: Vec<f64> = values
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|v| v.is_finite())
                    .collect();
                all_finite.extend(&finite);
                per_series_finite.push(finite);
            }
            if all_finite.is_empty() {
                return Err(FrameError::CompatibilityRejected(
                    "kde plot requires at least one finite numeric value".to_owned(),
                ));
            }
            let (data_min, data_max) = all_finite
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &v| {
                    (mn.min(v), mx.max(v))
                });
            let span = (data_max - data_min).max(1.0);
            let eval_min = data_min - 0.2 * span;
            let eval_max = data_max + 0.2 * span;
            let grid_size = 100;
            let step = (eval_max - eval_min) / (grid_size - 1) as f64;

            let mut curves: Vec<Vec<(f64, f64)>> = Vec::new();
            let mut max_density = 0.0f64;
            let inv_sqrt_2pi = 1.0 / (2.0 * std::f64::consts::PI).sqrt();

            for finite in &per_series_finite {
                let m = finite.len();
                if m == 0 {
                    curves.push(Vec::new());
                    continue;
                }
                let mean = finite.iter().sum::<f64>() / m as f64;
                let var = if m > 1 {
                    finite.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (m - 1) as f64
                } else {
                    1.0
                };
                let std_dev = var.sqrt().max(1e-6);
                let bandwidth = (1.06 * std_dev * (m as f64).powf(-0.2)).max(1e-6);
                let two_h_sq = 2.0 * bandwidth * bandwidth;
                let norm = 1.0 / (m as f64 * bandwidth) * inv_sqrt_2pi;

                let mut curve = Vec::with_capacity(grid_size);
                for g in 0..grid_size {
                    let t = eval_min + g as f64 * step;
                    let mut d = 0.0;
                    for &x_val in finite {
                        let diff = t - x_val;
                        d += (-diff * diff / two_h_sq).exp();
                    }
                    d *= norm;
                    max_density = max_density.max(d);
                    curve.push((t, d));
                }
                curves.push(curve);
            }
            if max_density <= 0.0 {
                max_density = 1.0;
            }

            let y_dens = |d: f64| MARGIN_TOP + (1.0 - d / (max_density * 1.05)) * PLOT_H;
            let x_dens = |t: f64| MARGIN_LEFT + (t - eval_min) / (eval_max - eval_min) * PLOT_W;

            for (si, curve) in curves.iter().enumerate() {
                if curve.is_empty() {
                    continue;
                }
                let color = palette(si);
                let points = curve
                    .iter()
                    .map(|(t, d)| format!("{:.2},{:.2}", x_dens(*t), y_dens(*d)))
                    .collect::<Vec<_>>()
                    .join(" ");
                let first_x = x_dens(curve[0].0);
                let last_x = x_dens(curve[curve.len() - 1].0);
                let base_y = y_dens(0.0);
                body.push_str(&format!(
                    "<polygon points=\"{first_x:.2},{base_y:.2} {points} {last_x:.2},{base_y:.2}\" fill=\"{color}\" fill-opacity=\"0.15\" stroke=\"none\"/>"
                ));
                body.push_str(&format!(
                    "<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"/>"
                ));
            }
        }
        PlotKind::Hexbin => {
            let pairs: Vec<(f64, f64)> = if is_xy_hexbin && views.len() >= 2 {
                let mut p = Vec::new();
                for g in 0..views.len() / 2 {
                    let v0 = &views[2 * g];
                    let v1 = &views[2 * g + 1];
                    let len = v0.len().min(v1.len());
                    for i in 0..len {
                        if let (Some(x), Some(y)) = (v0[i], v1[i])
                            && x.is_finite()
                            && y.is_finite()
                        {
                            p.push((x, y));
                        }
                    }
                }
                p
            } else if views.len() >= 2 {
                let v0 = &views[0];
                let v1 = &views[1];
                let len = v0.len().min(v1.len());
                (0..len)
                    .filter_map(|i| match (v0[i], v1[i]) {
                        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((x, y)),
                        _ => None,
                    })
                    .collect()
            } else if let Some(v0) = views.first() {
                v0.iter()
                    .enumerate()
                    .filter_map(|(i, v)| v.filter(|v| v.is_finite()).map(|v| (i as f64, v)))
                    .collect()
            } else {
                Vec::new()
            };

            if pairs.is_empty() {
                return Err(FrameError::CompatibilityRejected(
                    "hexbin plot requires numeric coordinate pairs".to_owned(),
                ));
            }

            let (min_x, max_x) = if let Some((ref x_sc, _)) = xy_scales {
                (x_sc.min, x_sc.max)
            } else {
                pairs
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), p| {
                        (mn.min(p.0), mx.max(p.0))
                    })
            };
            let (min_y, max_y) = if let Some((_, ref y_sc)) = xy_scales {
                (y_sc.min, y_sc.max)
            } else {
                pairs
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), p| {
                        (mn.min(p.1), mx.max(p.1))
                    })
            };
            let span_x = (max_x - min_x).max(1.0);
            let span_y = (max_y - min_y).max(1.0);
            let grid_nx = 16;
            let grid_ny = 12;
            let mut bins: std::collections::HashMap<(usize, usize), usize> =
                std::collections::HashMap::new();
            for &(px, py) in &pairs {
                let bx = (((px - min_x) / span_x * (grid_nx as f64 - 1.0)).round() as usize)
                    .min(grid_nx - 1);
                let by = (((py - min_y) / span_y * (grid_ny as f64 - 1.0)).round() as usize)
                    .min(grid_ny - 1);
                *bins.entry((bx, by)).or_insert(0) += 1;
            }
            let max_count = bins.values().copied().max().unwrap_or(1) as f64;
            let cell_w = PLOT_W / grid_nx as f64;
            let cell_h = PLOT_H / grid_ny as f64;

            for ((bx, by), count) in &bins {
                let cx = MARGIN_LEFT + (*bx as f64 + 0.5) * cell_w;
                let cy = MARGIN_TOP + (grid_ny - 1 - *by) as f64 * cell_h + cell_h * 0.5;
                let intensity = (*count as f64 / max_count).clamp(0.15, 1.0);
                let r = (cell_w.min(cell_h) * 0.45 * (0.4 + 0.6 * intensity)).max(2.0);
                body.push_str(&format!(
                    "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r:.2}\" fill=\"#4e79a7\" fill-opacity=\"{intensity:.2}\" stroke=\"#333\" stroke-width=\"0.5\"/>"
                ));
            }
        }
        PlotKind::Histogram | PlotKind::Box => unreachable!(),
    }

    let legend_entries: Vec<(String, &str)> = if spec.kind == PlotKind::Pie {
        if spec.series.len() == 1 {
            let s = &spec.series[0];
            let mut entries = Vec::new();
            let mut s_idx = 0;
            for (i, v) in views[0].iter().enumerate() {
                if let Some(val) = v
                    && val.is_finite()
                    && *val > 0.0
                {
                    let label = s
                        .index
                        .get(i)
                        .map(|lbl| crate::scalar_plot_label(&crate::index_label_to_scalar(lbl)))
                        .unwrap_or_else(|| i.to_string());
                    entries.push((label, palette(s_idx)));
                    s_idx += 1;
                }
            }
            entries
        } else if spec.series.iter().all(|s| s.values.len() == 1) {
            spec.series
                .iter()
                .enumerate()
                .map(|(i, s)| (s.name.clone(), palette(i)))
                .collect()
        } else {
            let mut entries = Vec::new();
            let mut s_idx = 0;
            for (si, s) in spec.series.iter().enumerate() {
                for (i, v) in views[si].iter().enumerate() {
                    if let Some(val) = v
                        && val.is_finite()
                        && *val > 0.0
                    {
                        let label = s
                            .index
                            .get(i)
                            .map(|lbl| {
                                format!(
                                    "{}[{}]",
                                    s.name,
                                    crate::scalar_plot_label(&crate::index_label_to_scalar(lbl))
                                )
                            })
                            .unwrap_or_else(|| format!("{}[{i}]", s.name));
                        entries.push((label, palette(s_idx)));
                        s_idx += 1;
                    }
                }
            }
            entries
        }
    } else if xy_scales.is_some() {
        let num_groups = spec.series.len() / 2;
        if num_groups <= 1 && !spec.method.contains("radviz") {
            vec![(
                format!("{} vs {}", spec.series[1].name, spec.series[0].name),
                palette(0),
            )]
        } else {
            (0..num_groups)
                .map(|g| {
                    let label = if let Some(ref gk) = spec.series[2 * g].group_key {
                        crate::group_key_label(gk)
                    } else if let Some(b_start) = spec.series[2 * g].name.find('[') {
                        let s = &spec.series[2 * g].name;
                        if let Some(b_end) = s.rfind(']') {
                            s[b_start + 1..b_end].to_owned()
                        } else {
                            s.clone()
                        }
                    } else {
                        format!("group {g}")
                    };
                    (label, palette(g))
                })
                .collect()
        }
    } else if spec.kind == PlotKind::Line
        && (spec.method.contains("parallel_coordinates")
            || spec.method.contains("andrews_curves"))
    {
        let mut unique_classes: Vec<String> = Vec::new();
        for s in &spec.series {
            let class_name = s
                .group_key
                .as_ref()
                .and_then(|gk| gk.first())
                .map(crate::scalar_plot_label)
                .unwrap_or_else(|| s.name.clone());
            if !unique_classes.contains(&class_name) {
                unique_classes.push(class_name);
            }
        }
        unique_classes
            .into_iter()
            .enumerate()
            .map(|(i, c)| (c, palette(i)))
            .collect()
    } else {
        spec.series
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), palette(i)))
            .collect()
    };
    let axes = if spec.kind == PlotKind::Pie {
        String::new()
    } else if let Some((ref x_sc, ref y_sc)) = xy_scales {
        svg_numeric_2d_axes(x_sc, y_sc)
    } else if spec.kind == PlotKind::Barh {
        svg_horizontal_axes(&scale, &x_labels)
    } else {
        svg_axes(&scale, &x_labels)
    };
    Ok(format!(
        "{}{body}{axes}{}</svg>",
        svg_open(&spec.method),
        legend(&legend_entries),
    ))
}

// ── Histogram primitives ───────────────────────────────────────────────────

fn histogram_body(
    series: &[PlotSeriesSpec],
    bins: usize,
    title: &str,
) -> Result<String, FrameError> {
    if bins == 0 {
        return Err(FrameError::CompatibilityRejected(
            "histogram requires at least one bin".to_owned(),
        ));
    }
    let views: Result<Vec<Vec<Option<f64>>>, FrameError> =
        series.iter().map(numeric_view).collect();
    let views = views?;
    let finite_series: Vec<Vec<f64>> = views
        .iter()
        .map(|v| v.iter().flatten().copied().collect())
        .collect();
    if finite_series.iter().all(|v| v.is_empty()) {
        return Err(FrameError::CompatibilityRejected(
            "no plottable (non-missing) values in this histogram".to_owned(),
        ));
    }

    let flat: Vec<Option<f64>> = finite_series
        .iter()
        .flat_map(|v| v.iter().copied())
        .map(Some)
        .collect();
    let scale = data_scale(&[flat])?;
    let step = (scale.max - scale.min) / bins as f64;

    let mut global_max = 0usize;
    let mut all_counts: Vec<Vec<usize>> = Vec::with_capacity(finite_series.len());
    for values in &finite_series {
        let mut counts = vec![0usize; bins];
        for v in values {
            let raw_idx = ((v - scale.min) / step).floor();
            let idx = if *v >= scale.max {
                bins - 1
            } else if raw_idx <= 0.0 {
                0
            } else {
                (raw_idx as usize).min(bins - 1)
            };
            counts[idx] += 1;
        }
        global_max = global_max.max(*counts.iter().max().unwrap_or(&0));
        all_counts.push(counts);
    }
    let global_max = global_max.max(1);

    let mut body = String::new();
    let width = PLOT_W / finite_series.len().max(1) as f64;
    for (si, counts) in all_counts.iter().enumerate() {
        let color = palette(si);
        let x0 = MARGIN_LEFT + si as f64 * width;
        let inner = width * 0.85;
        let bar_w = (inner / bins as f64 - 1.0).max(0.5);
        for (bi, count) in counts.iter().enumerate() {
            if *count == 0 {
                continue;
            }
            let h = *count as f64 / global_max as f64 * PLOT_H;
            let bx = x0 + bi as f64 * (inner / bins as f64);
            body.push_str(&format!(
                "<rect x=\"{bx:.2}\" y=\"{:.2}\" width=\"{bar_w:.2}\" height=\"{h:.2}\" fill=\"{color}\" stroke=\"white\"/>",
                MARGIN_TOP + PLOT_H - h
            ));
        }
    }

    let count_scale = Scale {
        min: 0.0,
        max: global_max as f64,
    };
    let mut x_labels = Vec::with_capacity(bins);
    for i in 0..bins {
        let bin_mid = scale.min + (i as f64 + 0.5) * step;
        x_labels.push(fmt_num(bin_mid));
    }
    let axes = svg_axes(&count_scale, &x_labels);
    let legend_entries: Vec<(String, &str)> = series
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.clone(), palette(i)))
        .collect();

    Ok(format!(
        "{}{body}{axes}{}</svg>",
        svg_open(title),
        legend(&legend_entries)
    ))
}

fn boxplot_body(series: &[PlotSeriesSpec], title: &str) -> Result<String, FrameError> {
    let views: Result<Vec<Vec<Option<f64>>>, FrameError> =
        series.iter().map(numeric_view).collect();
    let views = views?;
    let mut sorted: Vec<(usize, Vec<f64>)> = views
        .iter()
        .enumerate()
        .map(|(si, v)| {
            let mut vals: Vec<f64> = v.iter().flatten().copied().collect();
            vals.sort_by(f64::total_cmp);
            (si, vals)
        })
        .collect();
    sorted.retain(|(_, vals)| !vals.is_empty());
    if sorted.is_empty() {
        return Err(FrameError::CompatibilityRejected(
            "no plottable (non-missing) values in this boxplot".to_owned(),
        ));
    }
    let scale = data_scale(&views)?;
    let y = |v: f64| MARGIN_TOP + (scale.max - v) / (scale.max - scale.min) * PLOT_H;
    let slot = PLOT_W / sorted.len() as f64;
    let mut body = String::new();
    let mut x_labels = Vec::with_capacity(sorted.len());
    let mut legend_entries = Vec::with_capacity(sorted.len());
    for (box_idx, (orig_si, vals)) in sorted.iter().enumerate() {
        let color = palette(*orig_si);
        let name = &series[*orig_si].name;
        x_labels.push(name.clone());
        legend_entries.push((name.clone(), color));
        let (q1, med, q3) = (
            quantile(vals, 0.25),
            quantile(vals, 0.5),
            quantile(vals, 0.75),
        );
        let (lo, hi) = (vals[0], vals[vals.len() - 1]);
        let cx = MARGIN_LEFT + slot * (box_idx as f64 + 0.5);
        let bw = (slot * 0.5).max(1.0);
        let top = y(q3);
        let bottom = y(q1);
        body.push_str(&format!(
            "<line x1=\"{cx:.2}\" y1=\"{:.2}\" x2=\"{cx:.2}\" y2=\"{:.2}\" stroke=\"{color}\"/><line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\"/>",
            y(lo),
            y(q1),
            cx - bw / 2.0,
            y(lo),
            cx + bw / 2.0,
            y(lo),
        ));
        body.push_str(&format!(
            "<line x1=\"{cx:.2}\" y1=\"{:.2}\" x2=\"{cx:.2}\" y2=\"{:.2}\" stroke=\"{color}\"/><line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\"/>",
            y(q3),
            y(hi),
            cx - bw / 2.0,
            y(hi),
            cx + bw / 2.0,
            y(hi),
        ));
        body.push_str(&format!(
            "<rect x=\"{:.2}\" y=\"{top:.2}\" width=\"{bw:.2}\" height=\"{:.2}\" fill=\"{color}\" fill-opacity=\"0.35\" stroke=\"{color}\"/><line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\" stroke-width=\"2\"/>",
            cx - bw / 2.0,
            (bottom - top).abs().max(0.5),
            cx - bw / 2.0,
            y(med),
            cx + bw / 2.0,
            y(med),
        ));
    }
    let axes = svg_axes(&scale, &x_labels);
    Ok(format!(
        "{}{body}{axes}{}</svg>",
        svg_open(title),
        legend(&legend_entries)
    ))
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let h = (sorted.len() - 1) as f64 * q;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * (h - lo as f64)
}

// ── Public API ─────────────────────────────────────────────────────────────

impl PlotSpec {
    /// Render this spec to a deterministic SVG document (style floor: axes,
    /// ticks, gridlines, legend; see module docs). Fails closed on non-numeric
    /// series or all-missing data; never panics.
    pub fn to_svg(&self) -> Result<String, FrameError> {
        render_plot(self)
    }

    /// [`PlotSpec::to_svg`] as UTF-8 bytes (convenience for `write`/HTTP paths).
    pub fn to_svg_bytes(&self) -> Result<Vec<u8>, FrameError> {
        self.to_svg().map(String::into_bytes)
    }

    /// Wrap the deterministic SVG in an HTML figure container suitable for
    /// embedding into notebooks, dashboards, or web pages.
    pub fn to_html(&self) -> Result<String, FrameError> {
        self.to_svg().map(|s| wrap_svg_html(&s))
    }

    /// Render this spec to a complete, self-contained HTML5 page document.
    pub fn to_html_page(&self, title: Option<&str>) -> Result<String, FrameError> {
        let title_str = title.unwrap_or(&self.method);
        self.to_svg().map(|s| wrap_svg_html_page(&s, title_str))
    }

    /// Render this spec to Markdown-compatible HTML embedding.
    pub fn to_markdown(&self) -> Result<String, FrameError> {
        self.to_svg().map(|s| wrap_svg_markdown(&s))
    }

    /// Save the rendered plot to a file on disk. The file extension determines
    /// the target format: `.svg` (raw SVG XML), `.html` / `.htm` (standalone HTML
    /// document), or `.md` (markdown snippet).
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), FrameError> {
        let svg = self.to_svg()?;
        save_rendered_svg(&svg, path, &self.method)
    }
}

impl HistogramSpec {
    /// Render a grouped histogram: one inset of `bins` bars per series, NaN
    /// values skipped. Fails closed on non-numeric input or all-missing data.
    pub fn to_svg(&self) -> Result<String, FrameError> {
        let title = format!(
            "histogram ({})",
            self.series
                .first()
                .map(|s| s.name.as_str())
                .unwrap_or("values")
        );
        histogram_body(&self.series, self.bins, &title)
    }

    /// [`HistogramSpec::to_svg`] as UTF-8 bytes.
    pub fn to_svg_bytes(&self) -> Result<Vec<u8>, FrameError> {
        self.to_svg().map(String::into_bytes)
    }

    /// Wrap the deterministic SVG in an HTML figure container.
    pub fn to_html(&self) -> Result<String, FrameError> {
        self.to_svg().map(|s| wrap_svg_html(&s))
    }

    /// Render this histogram spec to a complete HTML5 page document.
    pub fn to_html_page(&self, title: Option<&str>) -> Result<String, FrameError> {
        let title_str = title.unwrap_or(&self.method);
        self.to_svg().map(|s| wrap_svg_html_page(&s, title_str))
    }

    /// Render this histogram spec to Markdown-compatible HTML embedding.
    pub fn to_markdown(&self) -> Result<String, FrameError> {
        self.to_svg().map(|s| wrap_svg_markdown(&s))
    }

    /// Save the rendered histogram to disk (`.svg`, `.html`/`.htm`, or `.md`).
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), FrameError> {
        let svg = self.to_svg()?;
        save_rendered_svg(&svg, path, &self.method)
    }
}

impl BoxPlotSpec {
    /// Render a side-by-side box-and-whisker plot (min, q1, median, q3, max —
    /// linear-interpolated quantiles, NaN skipped). Fails closed when every
    /// series is empty, or on non-numeric input.
    pub fn to_svg(&self) -> Result<String, FrameError> {
        let title = format!(
            "boxplot ({})",
            self.series
                .first()
                .map(|s| s.name.as_str())
                .unwrap_or("values")
        );
        boxplot_body(&self.series, &title)
    }

    /// [`BoxPlotSpec::to_svg`] as UTF-8 bytes.
    pub fn to_svg_bytes(&self) -> Result<Vec<u8>, FrameError> {
        self.to_svg().map(String::into_bytes)
    }

    /// Wrap the deterministic SVG in an HTML figure container.
    pub fn to_html(&self) -> Result<String, FrameError> {
        self.to_svg().map(|s| wrap_svg_html(&s))
    }

    /// Render this boxplot spec to a complete HTML5 page document.
    pub fn to_html_page(&self, title: Option<&str>) -> Result<String, FrameError> {
        let title_str = title.unwrap_or(&self.method);
        self.to_svg().map(|s| wrap_svg_html_page(&s, title_str))
    }

    /// Render this boxplot spec to Markdown-compatible HTML embedding.
    pub fn to_markdown(&self) -> Result<String, FrameError> {
        self.to_svg().map(|s| wrap_svg_markdown(&s))
    }

    /// Save the rendered boxplot to disk (`.svg`, `.html`/`.htm`, or `.md`).
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), FrameError> {
        let svg = self.to_svg()?;
        save_rendered_svg(&svg, path, &self.method)
    }
}

fn wrap_svg_html(svg: &str) -> String {
    format!(
        "<div class=\"frankenpandas-plot\" style=\"display:inline-block;max-width:100%;height:auto;\">\n{svg}\n</div>"
    )
}

fn wrap_svg_html_page(svg: &str, title: &str) -> String {
    let esc_title = esc(title);
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <title>{esc_title}</title>\n  <style>\n    body {{\n      margin: 0;\n      padding: 24px;\n      font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif;\n      background-color: #f8f9fa;\n      color: #212529;\n      display: flex;\n      justify-content: center;\n      align-items: center;\n      min-height: 100vh;\n    }}\n    .frankenpandas-plot-container {{\n      background: #ffffff;\n      padding: 20px;\n      border-radius: 8px;\n      box-shadow: 0 4px 12px rgba(0,0,0,0.08);\n      max-width: 100%;\n    }}\n    .frankenpandas-plot-container svg {{\n      display: block;\n      max-width: 100%;\n      height: auto;\n    }}\n  </style>\n</head>\n<body>\n  <div class=\"frankenpandas-plot-container\">\n    {svg}\n  </div>\n</body>\n</html>\n"
    )
}

fn wrap_svg_markdown(svg: &str) -> String {
    format!("<div class=\"frankenpandas-plot\">\n{svg}\n</div>\n")
}

fn save_rendered_svg<P: AsRef<std::path::Path>>(
    svg: &str,
    path: P,
    default_title: &str,
) -> Result<(), FrameError> {
    let path_ref = path.as_ref();
    let ext = path_ref
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let content = match ext.as_str() {
        "svg" => svg.as_bytes().to_vec(),
        "html" | "htm" => wrap_svg_html_page(svg, default_title).into_bytes(),
        "md" | "markdown" => wrap_svg_markdown(svg).into_bytes(),
        "png" | "pdf" | "jpg" | "jpeg" => {
            return Err(FrameError::CompatibilityRejected(format!(
                "saving plot directly to '.{ext}' requires an external rasterizer; supported vector formats are: .svg, .html, .htm, .md"
            )));
        }
        _ => svg.as_bytes().to_vec(),
    };

    if let Some(parent) = path_ref.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }

    std::fs::write(path_ref, content).map_err(|err| {
        FrameError::CompatibilityRejected(format!(
            "failed to save plot to '{}': {err}",
            path_ref.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BoxPlotSpec, FrameError, HistogramSpec, PlotKind, PlotSeriesSpec, PlotSpec, Scalar,
        numeric_view, quantile,
    };
    use crate::DType;

    fn series(name: &str, values: Vec<Scalar>) -> PlotSeriesSpec {
        PlotSeriesSpec {
            name: name.to_owned(),
            dtype: DType::Float64,
            index: (0..values.len() as i64)
                .map(crate::IndexLabel::Int64)
                .collect(),
            values,
            group_key: None,
        }
    }

    fn floats(values: &[f64]) -> Vec<Scalar> {
        values.iter().map(|v| Scalar::Float64(*v)).collect()
    }

    #[test]
    fn line_plot_renders_deterministic_svg_with_axis_and_legend() {
        let spec = PlotSpec {
            method: "plot".to_owned(),
            kind: PlotKind::Line,
            series: vec![series("s", floats(&[1.0, 2.0, 3.0]))],
        };
        let a = spec.to_svg().expect("render");
        let b = spec.to_svg().expect("render again");
        assert_eq!(a, b, "rendering must be deterministic");
        assert!(a.starts_with("<svg") && a.ends_with("</svg>"));
        assert!(a.contains("<polyline"), "line kind must draw polylines");
        assert!(a.contains(">s</text>"), "legend must carry the series name");
        assert!(a.contains(">plot</text>"), "title must carry the method");
    }

    #[test]
    fn nan_and_null_values_become_gaps_not_coordinates() {
        let spec = PlotSpec {
            method: "plot".to_owned(),
            kind: PlotKind::Line,
            series: vec![series(
                "gappy",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(f64::NAN),
                    Scalar::Float64(3.0),
                    Scalar::Null(crate::NullKind::NaN),
                    Scalar::Float64(5.0),
                ],
            )],
        };
        let svg = spec.to_svg().expect("render");
        assert!(svg.contains("<polyline"), "gaps still draw segments");
        // Two gaps split 5 values into 3 finite runs.
        assert_eq!(svg.matches("<polyline").count(), 3);
        assert!(
            !svg.contains("NaN"),
            "missing values must not leak as text coords"
        );
    }

    #[test]
    fn all_missing_and_non_numeric_series_fail_closed() {
        let all_nan = PlotSpec {
            method: "plot".to_owned(),
            kind: PlotKind::Line,
            series: vec![series("nan", vec![Scalar::Float64(f64::NAN); 3])],
        };
        assert!(matches!(
            all_nan.to_svg(),
            Err(FrameError::CompatibilityRejected(_))
        ));

        let utf8 = PlotSeriesSpec {
            name: "text".to_owned(),
            dtype: DType::Utf8,
            index: vec![crate::IndexLabel::Int64(0)],
            values: vec![Scalar::Utf8("nope".to_owned())],
            group_key: None,
        };
        let err = numeric_view(&utf8).expect_err("utf8 must fail closed");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));
    }

    #[test]
    fn histogram_bins_cover_every_finite_value_exactly_once() {
        let spec = HistogramSpec {
            method: "hist".to_owned(),
            bins: 4,
            series: vec![series("h", floats(&[0.0, 1.0, 2.0, 3.0, f64::NAN]))],
        };
        let svg = spec.to_svg().expect("render");
        // 4 non-missing values across 4 bins -> 4 bars (bars are the rects
        // with a white stroke; the legend + background rects are not).
        assert_eq!(
            svg.matches("stroke=\"white\"").count(),
            4,
            "one bar per populated bin"
        );
    }

    #[test]
    fn boxplot_quantiles_match_linear_interpolation() {
        let vals: Vec<f64> = (1..=5).map(f64::from).collect();
        assert_eq!(quantile(&vals, 0.5), 3.0);
        assert!((quantile(&vals, 0.25) - 2.0).abs() < 1e-9);
        assert!((quantile(&vals, 0.75) - 4.0).abs() < 1e-9);
        let even: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0];
        assert!((quantile(&even, 0.5) - 2.5).abs() < 1e-9);

        let spec = BoxPlotSpec {
            method: "boxplot".to_owned(),
            series: vec![series("b", floats(&[1.0, 2.0, 3.0, 4.0, 5.0]))],
        };
        let svg = spec.to_svg().expect("render");
        assert!(svg.contains("<rect"), "box body must be drawn");
    }

    #[test]
    fn xml_sensitive_names_are_escaped() {
        let spec = PlotSpec {
            method: "plot".to_owned(),
            kind: PlotKind::Line,
            series: vec![series("<b>&\"x'", floats(&[1.0, 2.0]))],
        };
        let svg = spec.to_svg().expect("render");
        assert!(
            !svg.contains("<b>&"),
            "raw XML-sensitive text must not leak"
        );
        assert!(svg.contains("&lt;b&gt;&amp;"), "name must be escaped");
    }

    #[test]
    fn svg_bytes_and_bar_and_pie_paths_render() {
        let bar = PlotSpec {
            method: "bar".to_owned(),
            kind: PlotKind::Bar,
            series: vec![series("b", floats(&[1.0, 5.0]))],
        };
        assert!(bar.to_svg().expect("bar").contains("<rect"));
        let bytes = bar.to_svg_bytes().expect("bytes");
        assert!(bytes.starts_with(b"<svg"));

        let pie = PlotSpec {
            method: "pie".to_owned(),
            kind: PlotKind::Pie,
            series: vec![series("p", floats(&[1.0, 1.0, 2.0]))],
        };
        assert!(pie.to_svg().expect("pie").contains("<path"));

        let negative_pie = PlotSpec {
            method: "pie".to_owned(),
            kind: PlotKind::Pie,
            series: vec![series("p", floats(&[-1.0, 2.0]))],
        };
        assert!(matches!(
            negative_pie.to_svg(),
            Err(FrameError::CompatibilityRejected(_))
        ));
    }

    #[test]
    fn pie_chart_handles_nan_distinct_colors_and_full_circle() {
        // 1. Nan in pie values must be skipped, not produce NaN in path coordinates.
        let pie_nan = PlotSpec {
            method: "pie".to_owned(),
            kind: PlotKind::Pie,
            series: vec![series(
                "p",
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(f64::NAN),
                    Scalar::Float64(20.0),
                ],
            )],
        };
        let svg_nan = pie_nan.to_svg().expect("pie with nan renders");
        assert!(
            !svg_nan.contains("NaN"),
            "pie svg must never contain NaN coordinates"
        );
        assert!(svg_nan.contains("<path"), "pie wedges must be drawn");

        // 2. Multi-slice pie must use distinct palette colors for slices, not monochrome.
        let pie_multi = PlotSpec {
            method: "pie".to_owned(),
            kind: PlotKind::Pie,
            series: vec![series("p", floats(&[10.0, 20.0]))],
        };
        let svg_multi = pie_multi.to_svg().expect("multi-slice pie renders");
        assert!(svg_multi.contains(super::palette(0)));
        assert!(svg_multi.contains(super::palette(1)));
        // Pie charts must not render Cartesian axes or gridlines
        assert!(
            !svg_multi.contains("<line x1=\"56\" y1=\"30\""),
            "pie charts must not render cartesian axes"
        );

        // 3. Single-slice (100%) pie must render a full circle rather than degenerate 0-length arc.
        let pie_single = PlotSpec {
            method: "pie".to_owned(),
            kind: PlotKind::Pie,
            series: vec![series("p", floats(&[100.0]))],
        };
        let svg_single = pie_single.to_svg().expect("single-slice pie renders");
        assert!(
            svg_single.contains("<circle"),
            "100% single slice must render circle"
        );
    }

    #[test]
    fn export_formats_html_page_markdown_and_file_save() {
        let spec = PlotSpec {
            method: "test_export".to_owned(),
            kind: PlotKind::Line,
            series: vec![series("y", floats(&[1.0, 2.0, 3.0]))],
        };

        // 1. to_html wraps in container
        let html = spec.to_html().expect("to_html");
        assert!(html.contains("<div class=\"frankenpandas-plot\""));
        assert!(html.contains("<svg"));

        // 2. to_html_page produces valid HTML5 document
        let page = spec.to_html_page(Some("My Title")).expect("to_html_page");
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(page.contains("<title>My Title</title>"));
        assert!(page.contains("<svg"));

        // 3. to_markdown embeds in div container
        let md = spec.to_markdown().expect("to_markdown");
        assert!(md.contains("<div class=\"frankenpandas-plot\">"));
        assert!(md.contains("<svg"));

        // 4. save to tempdir
        let temp_dir = std::env::temp_dir().join(format!("fp_test_plot_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let svg_path = temp_dir.join("plot.svg");
        spec.save(&svg_path).expect("save svg");
        let read_svg = std::fs::read_to_string(&svg_path).expect("read saved svg");
        assert!(read_svg.starts_with("<svg"));

        let html_path = temp_dir.join("plot.html");
        spec.save(&html_path).expect("save html");
        let read_html = std::fs::read_to_string(&html_path).expect("read saved html");
        assert!(read_html.starts_with("<!DOCTYPE html>"));

        let md_path = temp_dir.join("plot.md");
        spec.save(&md_path).expect("save md");
        let read_md = std::fs::read_to_string(&md_path).expect("read saved md");
        assert!(read_md.contains("<div class=\"frankenpandas-plot\">"));

        // 5. save to unsupported raster format fails closed
        let png_path = temp_dir.join("plot.png");
        let err = spec.save(&png_path).expect_err("png must fail closed");
        assert!(matches!(err, FrameError::CompatibilityRejected(_)));

        // Clean up temp test files
        let _ = std::fs::remove_file(&svg_path);
        let _ = std::fs::remove_file(&html_path);
        let _ = std::fs::remove_file(&md_path);
        let _ = std::fs::remove_dir(&temp_dir);

        // 6. Histogram and BoxPlot export methods
        let hist = HistogramSpec {
            method: "hist".to_owned(),
            bins: 3,
            series: vec![series("h", floats(&[1.0, 2.0, 3.0]))],
        };
        assert!(
            hist.to_html()
                .expect("hist html")
                .contains("<div class=\"frankenpandas-plot\"")
        );
        assert!(
            hist.to_html_page(None)
                .expect("hist page")
                .contains("<!DOCTYPE html>")
        );
        assert!(
            hist.to_markdown()
                .expect("hist md")
                .contains("<div class=\"frankenpandas-plot\">")
        );

        let bplot = BoxPlotSpec {
            method: "bplot".to_owned(),
            series: vec![series("b", floats(&[1.0, 2.0, 3.0]))],
        };
        assert!(
            bplot
                .to_html()
                .expect("bplot html")
                .contains("<div class=\"frankenpandas-plot\"")
        );
        assert!(
            bplot
                .to_html_page(None)
                .expect("bplot page")
                .contains("<!DOCTYPE html>")
        );
        assert!(
            bplot
                .to_markdown()
                .expect("bplot md")
                .contains("<div class=\"frankenpandas-plot\">")
        );
    }

    #[test]
    fn infinite_and_neg_infinite_values_become_gaps_not_coordinates() {
        let spec = PlotSpec {
            method: "plot_gap_test".to_owned(),
            kind: PlotKind::Line,
            series: vec![series(
                "s_test",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(f64::INFINITY),
                    Scalar::Float64(3.0),
                    Scalar::Float64(f64::NEG_INFINITY),
                    Scalar::Float64(5.0),
                ],
            )],
        };
        let svg = spec.to_svg().expect("inf values must render as gaps");
        assert!(
            !svg.contains("inf"),
            "SVG must not contain 'inf' coordinates: {svg}"
        );
        assert!(
            !svg.contains("-inf"),
            "SVG must not contain '-inf' coordinates: {svg}"
        );
        assert_eq!(
            svg.matches("<polyline").count(),
            3,
            "2 infs split 5 items into 3 runs"
        );
    }

    #[test]
    fn bar_plot_supports_negative_values_and_zero_baseline() {
        let spec = PlotSpec {
            method: "bar_neg".to_owned(),
            kind: PlotKind::Bar,
            series: vec![series(
                "b",
                vec![
                    Scalar::Float64(-10.0),
                    Scalar::Float64(0.0),
                    Scalar::Float64(20.0),
                ],
            )],
        };
        let svg = spec.to_svg().expect("bar plot with negatives renders");
        assert!(svg.contains("<rect"), "bars must render as rects");
        assert!(
            !svg.contains("height=\"-"),
            "rect height must never be negative"
        );
    }

    #[test]
    fn plot_kind_box_and_histogram_render_correct_primitives() {
        // PlotKind::Box on PlotSpec must render boxplot primitives (rect + line whiskers)
        let box_spec = PlotSpec {
            method: "box_kind".to_owned(),
            kind: PlotKind::Box,
            series: vec![series("box_s", floats(&[1.0, 2.0, 3.0, 4.0, 5.0]))],
        };
        let svg_box = box_spec.to_svg().expect("box kind renders");
        assert!(svg_box.contains("<rect"), "boxplot must have box rect");
        assert!(svg_box.contains("<line"), "boxplot must have whisker lines");
        assert!(svg_box.contains("box_s"), "boxplot must carry series name");

        // PlotKind::Histogram on PlotSpec must render histogram primitives
        let hist_spec = PlotSpec {
            method: "hist_kind".to_owned(),
            kind: PlotKind::Histogram,
            series: vec![series("hist_s", floats(&[1.0, 2.0, 2.5, 3.0]))],
        };
        let svg_hist = hist_spec.to_svg().expect("hist kind renders");
        assert!(svg_hist.contains("<rect"), "hist must have bars");
        assert!(svg_hist.contains("hist_s"), "hist must carry series name");
    }

    #[test]
    fn legend_overflow_handles_more_than_six_series() {
        let mut s_vec = Vec::new();
        for i in 0..8 {
            s_vec.push(series(&format!("col_{i}"), floats(&[1.0, 2.0])));
        }
        let spec = PlotSpec {
            method: "multi".to_owned(),
            kind: PlotKind::Line,
            series: s_vec,
        };
        let svg = spec.to_svg().expect("multi series renders");
        assert!(
            svg.contains("+3 more"),
            "overflow legend must display +3 more"
        );
    }

    #[test]
    fn barh_kde_density_hexbin_render_expected_svg_elements() {
        // 1. Barh
        let barh_spec = PlotSpec {
            method: "test_barh".to_owned(),
            kind: PlotKind::Barh,
            series: vec![series("bh", floats(&[10.0, 20.0, 15.0]))],
        };
        let barh_svg = barh_spec.to_svg().expect("barh renders");
        assert!(barh_svg.contains("<rect"), "barh must contain rects");
        assert!(
            barh_svg.contains("bh"),
            "barh must contain legend series name"
        );

        // 2. KDE & Density
        let kde_spec = PlotSpec {
            method: "test_kde".to_owned(),
            kind: PlotKind::Kde,
            series: vec![series("kd", floats(&[1.0, 2.0, 2.5, 3.0, 4.0, 5.0]))],
        };
        let kde_svg = kde_spec.to_svg().expect("kde renders");
        assert!(
            kde_svg.contains("<polyline"),
            "kde must contain polyline curve"
        );
        assert!(kde_svg.contains("<polygon"), "kde must contain filled area");

        let density_spec = PlotSpec {
            method: "test_density".to_owned(),
            kind: PlotKind::Density,
            series: vec![series("dens", floats(&[1.0, 2.0, 2.5, 3.0, 4.0, 5.0]))],
        };
        let dens_svg = density_spec.to_svg().expect("density renders");
        assert!(
            dens_svg.contains("<polyline"),
            "density must contain polyline curve"
        );

        // 3. Hexbin
        let hexbin_spec = PlotSpec {
            method: "test_hexbin".to_owned(),
            kind: PlotKind::Hexbin,
            series: vec![
                series("x", floats(&[1.0, 2.0, 3.0, 4.0, 5.0])),
                series("y", floats(&[2.0, 4.0, 6.0, 8.0, 10.0])),
            ],
        };
        let hex_svg = hexbin_spec.to_svg().expect("hexbin renders");
        assert!(
            hex_svg.contains("<circle"),
            "hexbin must contain binned circles"
        );
    }

    #[test]
    fn scatter_xy_renders_numeric_2d_axes_and_mapped_coordinates() {
        let spec = PlotSpec {
            method: "DataFrame.plot.scatter(x='wt', y='mpg')".to_owned(),
            kind: PlotKind::Scatter,
            series: vec![
                series("wt", floats(&[10.0, 20.0, 30.0])),
                series("mpg", floats(&[100.0, 200.0, 300.0])),
            ],
        };
        let svg = spec.to_svg().expect("xy scatter renders");
        // Must contain circles for data points
        assert!(svg.contains("<circle"));
        assert_eq!(svg.matches("<circle").count(), 3);
        // Point (10.0, 100.0) -> left (56.00), bottom (358.00)
        assert!(
            svg.contains("cx=\"56.00\" cy=\"358.00\""),
            "lower-left point must map to (56.00, 358.00): {svg}"
        );
        // Point (30.0, 300.0) -> right (624.00), top (30.00)
        assert!(
            svg.contains("cx=\"624.00\" cy=\"30.00\""),
            "upper-right point must map to (624.00, 30.00): {svg}"
        );
        // Point (20.0, 200.0) -> center (340.00, 194.00)
        assert!(
            svg.contains("cx=\"340.00\" cy=\"194.00\""),
            "center point must map to (340.00, 194.00): {svg}"
        );
        // Must contain numeric 2D tick labels for both axes
        assert!(svg.contains(">10</text>"));
        assert!(svg.contains(">30</text>"));
        assert!(svg.contains(">100</text>"));
        assert!(svg.contains(">300</text>"));
        // Legend must display "mpg vs wt"
        assert!(svg.contains("mpg vs wt"));
    }

    #[test]
    fn hexbin_xy_renders_numeric_2d_axes_and_legend() {
        let spec = PlotSpec {
            method: "DataFrame.plot.hexbin(x='a', y='b')".to_owned(),
            kind: PlotKind::Hexbin,
            series: vec![
                series("a", floats(&[1.0, 2.0, 3.0, 4.0, 5.0])),
                series("b", floats(&[10.0, 20.0, 30.0, 40.0, 50.0])),
            ],
        };
        let svg = spec.to_svg().expect("xy hexbin renders");
        assert!(svg.contains("<circle"));
        assert!(svg.contains("b vs a"));
        assert!(svg.contains(">1</text>"));
        assert!(svg.contains(">5</text>"));
        assert!(svg.contains(">10</text>"));
        assert!(svg.contains(">50</text>"));
    }

    #[test]
    fn dataframe_scatter_and_hexbin_xy_methods() {
        use crate::{DataFrame, IndexLabel, Series};
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let sx = Series::from_values("x", labels.clone(), floats(&[10.0, 20.0, 30.0])).unwrap();
        let sy = Series::from_values("y", labels, floats(&[100.0, 200.0, 300.0])).unwrap();
        let df = DataFrame::from_series(vec![sx, sy]).unwrap();

        // 1. scatter_xy
        let sc_spec = df.scatter_xy("x", "y").expect("scatter_xy");
        assert_eq!(sc_spec.kind, PlotKind::Scatter);
        assert_eq!(sc_spec.method, "DataFrame.plot.scatter(x='x', y='y')");
        let svg = df.scatter_xy_to_svg("x", "y").expect("scatter_xy_to_svg");
        assert!(svg.contains("<circle"));
        let html = df.scatter_xy_to_html("x", "y").expect("scatter_xy_to_html");
        assert!(html.contains("<svg"));

        // 2. hexbin_xy & hexbin_columns
        let hb_spec = df.hexbin_xy("x", "y").expect("hexbin_xy");
        assert_eq!(hb_spec.kind, PlotKind::Hexbin);
        let hb_svg = df.hexbin_xy_to_svg("x", "y").expect("hexbin_xy_to_svg");
        assert!(hb_svg.contains("<circle"));
        let hb_html = df.hexbin_xy_to_html("x", "y").expect("hexbin_xy_to_html");
        assert!(hb_html.contains("<svg"));

        // 3. plot_xy
        let p_sc = df
            .plot_xy(PlotKind::Scatter, "x", "y")
            .expect("plot_xy scatter");
        assert_eq!(p_sc.kind, PlotKind::Scatter);
        let p_hb = df
            .plot_xy(PlotKind::Hexbin, "x", "y")
            .expect("plot_xy hexbin");
        assert_eq!(p_hb.kind, PlotKind::Hexbin);
        let p_line = df.plot_xy(PlotKind::Line, "x", "y").expect("plot_xy line");
        assert_eq!(p_line.kind, PlotKind::Line);
        assert_eq!(p_line.series.len(), 2);
    }

    #[test]
    fn dataframe_hist_and_boxplot_columns_and_by() {
        use crate::{DataFrame, IndexLabel, Series};
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ];
        let sa = Series::from_values("a", labels.clone(), floats(&[1.0, 2.0, 3.0, 4.0])).unwrap();
        let sb =
            Series::from_values("b", labels.clone(), floats(&[10.0, 20.0, 30.0, 40.0])).unwrap();
        let sc = Series::from_values(
            "category",
            labels.clone(),
            vec![
                Scalar::Utf8("cat1".to_string()),
                Scalar::Utf8("cat2".to_string()),
                Scalar::Utf8("cat1".to_string()),
                Scalar::Utf8("cat2".to_string()),
            ],
        )
        .unwrap();
        let df = DataFrame::from_series(vec![sa, sb, sc]).unwrap();

        // 1. hist_columns
        let h_cols = df.hist_columns(&["a", "b"], 5).expect("hist_columns");
        assert_eq!(h_cols.method, "DataFrame.hist(column=[\"a\", \"b\"])");
        assert_eq!(h_cols.bins, 5);
        assert_eq!(h_cols.series.len(), 2);
        let h_svg = df
            .hist_columns_to_svg(&["a", "b"], 5)
            .expect("hist_columns_to_svg");
        assert!(h_svg.contains("<svg"));
        let h_html = df
            .hist_columns_to_html(&["a", "b"], 5)
            .expect("hist_columns_to_html");
        assert!(h_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(h_html.contains("<svg"));

        // 2. hist_by
        let h_by = df.hist_by("a", "category", 4).expect("hist_by");
        assert_eq!(h_by.method, "DataFrame.hist(column='a', by='category')");
        assert_eq!(h_by.bins, 4);
        assert_eq!(h_by.series.len(), 2); // cat1 and cat2
        let h_by_svg = df
            .hist_by_to_svg("a", "category", 4)
            .expect("hist_by_to_svg");
        assert!(h_by_svg.contains("<svg"));
        let h_by_html = df
            .hist_by_to_html("a", "category", 4)
            .expect("hist_by_to_html");
        assert!(h_by_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(h_by_html.contains("<svg"));

        // 3. boxplot_columns
        let b_cols = df.boxplot_columns(&["a", "b"]).expect("boxplot_columns");
        assert_eq!(b_cols.method, "DataFrame.boxplot(column=[\"a\", \"b\"])");
        assert_eq!(b_cols.series.len(), 2);
        let b_svg = df
            .boxplot_columns_to_svg(&["a", "b"])
            .expect("boxplot_columns_to_svg");
        assert!(b_svg.contains("<svg"));
        let b_html = df
            .boxplot_columns_to_html(&["a", "b"])
            .expect("boxplot_columns_to_html");
        assert!(b_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(b_html.contains("<svg"));

        // 4. boxplot_by
        let b_by = df.boxplot_by("b", "category").expect("boxplot_by");
        assert_eq!(b_by.method, "DataFrame.boxplot(column='b', by='category')");
        assert_eq!(b_by.series.len(), 2); // cat1 and cat2
        let b_by_svg = df
            .boxplot_by_to_svg("b", "category")
            .expect("boxplot_by_to_svg");
        assert!(b_by_svg.contains("<svg"));
        let b_by_html = df
            .boxplot_by_to_html("b", "category")
            .expect("boxplot_by_to_html");
        assert!(b_by_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(b_by_html.contains("<svg"));
    }

    #[test]
    fn dataframe_groupby_scatter_and_hexbin() {
        use crate::{DataFrame, IndexLabel, Series};
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ];
        let sx = Series::from_values("x", labels.clone(), floats(&[1.0, 2.0, 3.0, 4.0])).unwrap();
        let sy =
            Series::from_values("y", labels.clone(), floats(&[10.0, 20.0, 30.0, 40.0])).unwrap();
        let sg = Series::from_values(
            "group",
            labels.clone(),
            vec![
                Scalar::Utf8("alpha".to_string()),
                Scalar::Utf8("beta".to_string()),
                Scalar::Utf8("alpha".to_string()),
                Scalar::Utf8("beta".to_string()),
            ],
        )
        .unwrap();
        let df = DataFrame::from_series(vec![sx, sy, sg]).unwrap();
        let gb = df.groupby(&["group"]).unwrap();

        // 1. scatter_columns and scatter_xy
        let sc_spec = gb.scatter_columns("x", "y").expect("gb scatter_columns");
        assert_eq!(sc_spec.kind, PlotKind::Scatter);
        assert_eq!(
            sc_spec.method,
            "DataFrameGroupBy.plot.scatter(x='x', y='y')"
        );
        assert_eq!(sc_spec.series.len(), 4); // 2 groups * 2 cols (x, y)
        let sc_svg = gb
            .scatter_columns_to_svg("x", "y")
            .expect("gb scatter_columns_to_svg");
        assert!(sc_svg.contains("<svg"));
        assert!(sc_svg.contains("<circle"));
        let sc_html = gb
            .scatter_xy_to_html("x", "y")
            .expect("gb scatter_xy_to_html");
        assert!(sc_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(sc_html.contains("<svg"));

        // 2. hexbin_columns and hexbin_xy
        let hb_spec = gb.hexbin_columns("x", "y").expect("gb hexbin_columns");
        assert_eq!(hb_spec.kind, PlotKind::Hexbin);
        assert_eq!(hb_spec.method, "DataFrameGroupBy.plot.hexbin(x='x', y='y')");
        assert_eq!(hb_spec.series.len(), 4);
        let hb_svg = gb
            .hexbin_columns_to_svg("x", "y")
            .expect("gb hexbin_columns_to_svg");
        assert!(hb_svg.contains("<svg"));
        assert!(hb_svg.contains("<circle"));
        let hb_html = gb
            .hexbin_xy_to_html("x", "y")
            .expect("gb hexbin_xy_to_html");
        assert!(hb_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(hb_html.contains("<svg"));

        // 3. plot_xy
        let p_sc = gb
            .plot_xy(PlotKind::Scatter, "x", "y")
            .expect("gb plot_xy scatter");
        assert_eq!(p_sc.kind, PlotKind::Scatter);
        assert_eq!(p_sc.series.len(), 4);

        let p_hb = gb
            .plot_xy(PlotKind::Hexbin, "x", "y")
            .expect("gb plot_xy hexbin");
        assert_eq!(p_hb.kind, PlotKind::Hexbin);
        assert_eq!(p_hb.series.len(), 4);

        // Invalid plot_xy kind error check
        let err = gb.plot_xy(PlotKind::Line, "x", "y");
        assert!(err.is_err());
    }

    #[test]
    fn dataframe_and_groupby_hist_and_boxplot_ergonomics() {
        use crate::{DataFrame, IndexLabel, Series};
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ];
        let sx = Series::from_values("x", labels.clone(), floats(&[1.0, 2.0, 3.0, 4.0])).unwrap();
        let sy =
            Series::from_values("y", labels.clone(), floats(&[10.0, 20.0, 30.0, 40.0])).unwrap();
        let sg = Series::from_values(
            "group",
            labels.clone(),
            vec![
                Scalar::Utf8("alpha".to_string()),
                Scalar::Utf8("beta".to_string()),
                Scalar::Utf8("alpha".to_string()),
                Scalar::Utf8("beta".to_string()),
            ],
        )
        .unwrap();
        let df = DataFrame::from_series(vec![sx.clone(), sy.clone(), sg.clone()]).unwrap();
        let df_num = DataFrame::from_series(vec![sx.clone(), sy.clone()]).unwrap();
        let gb = df.groupby(&["group"]).unwrap();
        let by_s = sg.clone();
        let ser_gb = sx.groupby(&by_s).unwrap();

        let temp_dir =
            std::env::temp_dir().join(format!("fp_test_hist_box_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. DataFrame hist_with_bins (svg, html, file)
        let df_h_svg = df_num
            .hist_with_bins_to_svg(6)
            .expect("df hist_with_bins_to_svg");
        assert!(df_h_svg.contains("<svg"));
        assert!(df_h_svg.contains("<rect"));
        let df_h_html = df_num
            .hist_with_bins_to_html(6)
            .expect("df hist_with_bins_to_html");
        assert!(df_h_html.contains("<div class=\"frankenpandas-plot\""));
        assert!(df_h_html.contains("<svg"));
        let df_h_file = temp_dir.join("df_hist.svg");
        df_num
            .hist_with_bins_to_file(6, &df_h_file)
            .expect("df hist_with_bins_to_file");
        assert!(std::fs::read_to_string(&df_h_file)
            .expect("read df_hist")
            .contains("<svg"));

        // 2. DataFrame hist_columns & boxplot_columns (svg, html, file)
        let df_hc_svg = df
            .hist_columns_to_svg(&["x", "y"], 5)
            .expect("df hist_columns_to_svg");
        assert!(df_hc_svg.contains("<svg"));
        let df_hc_html = df
            .hist_columns_to_html(&["x", "y"], 5)
            .expect("df hist_columns_to_html");
        assert!(df_hc_html.contains("<div class=\"frankenpandas-plot\""));
        let df_hc_file = temp_dir.join("df_hist_cols.svg");
        df.hist_columns_to_file(&["x", "y"], 5, &df_hc_file)
            .expect("df hist_columns_to_file");
        assert!(std::fs::read_to_string(&df_hc_file)
            .expect("read df_hist_cols")
            .contains("<svg"));

        let df_bc_svg = df
            .boxplot_columns_to_svg(&["x"])
            .expect("df boxplot_columns_to_svg");
        assert!(df_bc_svg.contains("<svg"));
        assert!(df_bc_svg.contains("<line"));
        let df_bc_html = df
            .boxplot_columns_to_html(&["x"])
            .expect("df boxplot_columns_to_html");
        assert!(df_bc_html.contains("<div class=\"frankenpandas-plot\""));
        let df_bc_file = temp_dir.join("df_box_cols.svg");
        df.boxplot_columns_to_file(&["x"], &df_bc_file)
            .expect("df boxplot_columns_to_file");
        assert!(std::fs::read_to_string(&df_bc_file)
            .expect("read df_box_cols")
            .contains("<svg"));

        // 3. Series hist_with_bins (svg, html, file)
        let s_h_svg = sx.hist_with_bins_to_svg(4).expect("s hist_with_bins_to_svg");
        assert!(s_h_svg.contains("<svg"));
        let s_h_html = sx.hist_with_bins_to_html(4).expect("s hist_with_bins_to_html");
        assert!(s_h_html.contains("<div class=\"frankenpandas-plot\""));
        let s_h_file = temp_dir.join("s_hist.svg");
        sx.hist_with_bins_to_file(4, &s_h_file)
            .expect("s hist_with_bins_to_file");
        assert!(std::fs::read_to_string(&s_h_file)
            .expect("read s_hist")
            .contains("<svg"));

        // 4. DataFrameGroupBy hist_with_bins (svg, html, file)
        let gb_h_svg = gb.hist_with_bins_to_svg(5).expect("gb hist_with_bins_to_svg");
        assert!(gb_h_svg.contains("<svg"));
        let gb_h_html = gb
            .hist_with_bins_to_html(5)
            .expect("gb hist_with_bins_to_html");
        assert!(gb_h_html.contains("<div class=\"frankenpandas-plot\""));
        let gb_h_file = temp_dir.join("gb_hist.svg");
        gb.hist_with_bins_to_file(5, &gb_h_file)
            .expect("gb hist_with_bins_to_file");
        assert!(std::fs::read_to_string(&gb_h_file)
            .expect("read gb_hist")
            .contains("<svg"));

        // 5. DataFrameGroupBy hist_columns & boxplot_columns (svg, html, file)
        let gb_hc_svg = gb
            .hist_columns_to_svg(&["x", "y"], 5)
            .expect("gb hist_columns_to_svg");
        assert!(gb_hc_svg.contains("<svg"));
        let gb_hc_html = gb
            .hist_columns_to_html(&["x", "y"], 5)
            .expect("gb hist_columns_to_html");
        assert!(gb_hc_html.contains("<div class=\"frankenpandas-plot\""));
        let gb_hc_file = temp_dir.join("gb_hist_cols.svg");
        gb.hist_columns_to_file(&["x", "y"], 5, &gb_hc_file)
            .expect("gb hist_columns_to_file");
        assert!(std::fs::read_to_string(&gb_hc_file)
            .expect("read gb_hist_cols")
            .contains("<svg"));

        let gb_bc_svg = gb
            .boxplot_columns_to_svg(&["x"])
            .expect("gb boxplot_columns_to_svg");
        assert!(gb_bc_svg.contains("<svg"));
        assert!(gb_bc_svg.contains("<line"));
        let gb_bc_html = gb
            .boxplot_columns_to_html(&["x"])
            .expect("gb boxplot_columns_to_html");
        assert!(gb_bc_html.contains("<div class=\"frankenpandas-plot\""));
        let gb_bc_file = temp_dir.join("gb_box_cols.svg");
        gb.boxplot_columns_to_file(&["x"], &gb_bc_file)
            .expect("gb boxplot_columns_to_file");
        assert!(std::fs::read_to_string(&gb_bc_file)
            .expect("read gb_box_cols")
            .contains("<svg"));

        // 6. SeriesGroupBy hist_with_bins (svg, html, file)
        let sgb_h_svg = ser_gb
            .hist_with_bins_to_svg(4)
            .expect("sgb hist_with_bins_to_svg");
        assert!(sgb_h_svg.contains("<svg"));
        let sgb_h_html = ser_gb
            .hist_with_bins_to_html(4)
            .expect("sgb hist_with_bins_to_html");
        assert!(sgb_h_html.contains("<div class=\"frankenpandas-plot\""));
        let sgb_h_file = temp_dir.join("sgb_hist.svg");
        ser_gb
            .hist_with_bins_to_file(4, &sgb_h_file)
            .expect("sgb hist_with_bins_to_file");
        assert!(std::fs::read_to_string(&sgb_h_file)
            .expect("read sgb_hist")
            .contains("<svg"));

        // 7. Error handling for missing columns
        assert!(gb.hist_columns(&["nonexistent"], 5).is_err());
        assert!(gb.boxplot_columns(&["nonexistent"]).is_err());
        assert!(df.hist_columns(&["nonexistent"], 5).is_err());
        assert!(df.boxplot_columns(&["nonexistent"]).is_err());

        // Cleanup temp files
        let _ = std::fs::remove_file(&df_h_file);
        let _ = std::fs::remove_file(&df_hc_file);
        let _ = std::fs::remove_file(&df_bc_file);
        let _ = std::fs::remove_file(&s_h_file);
        let _ = std::fs::remove_file(&gb_h_file);
        let _ = std::fs::remove_file(&gb_hc_file);
        let _ = std::fs::remove_file(&gb_bc_file);
        let _ = std::fs::remove_file(&sgb_h_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    fn lag_plot_and_autocorrelation_plot_and_grouped_plots() {
        use crate::{DataFrame, IndexLabel, Series};
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
            IndexLabel::Int64(4),
            IndexLabel::Int64(5),
        ];
        let sx = Series::from_values(
            "val",
            labels.clone(),
            floats(&[1.0, 2.0, 3.0, 2.0, 1.0, 2.0]),
        )
        .unwrap();
        let sy = Series::from_values(
            "val2",
            labels.clone(),
            floats(&[10.0, 20.0, 15.0, 25.0, 30.0, 20.0]),
        )
        .unwrap();
        let sg = Series::from_values(
            "group",
            labels.clone(),
            vec![
                Scalar::Utf8("A".to_string()),
                Scalar::Utf8("B".to_string()),
                Scalar::Utf8("A".to_string()),
                Scalar::Utf8("B".to_string()),
                Scalar::Utf8("A".to_string()),
                Scalar::Utf8("B".to_string()),
            ],
        )
        .unwrap();

        let df = DataFrame::from_series(vec![sx.clone(), sy.clone(), sg.clone()]).unwrap();

        let temp_dir =
            std::env::temp_dir().join(format!("fp_test_lag_ac_grouped_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. Series lag_plot
        let lag_spec = sx.lag_plot(1).expect("lag_plot(1)");
        assert_eq!(lag_spec.kind, PlotKind::Scatter);
        assert_eq!(lag_spec.series.len(), 2);
        assert_eq!(lag_spec.series[0].name, "y(t)");
        assert_eq!(lag_spec.series[1].name, "y(t+1)");
        let lag_svg = sx.lag_plot_to_svg(1).expect("lag_plot_to_svg");
        assert!(lag_svg.contains("<svg"));
        assert!(lag_svg.contains("<circle"));
        let lag_html = sx.lag_plot_to_html(1).expect("lag_plot_to_html");
        assert!(lag_html.contains("<div class=\"frankenpandas-plot\""));
        let lag_file = temp_dir.join("lag.svg");
        sx.lag_plot_to_file(1, &lag_file).expect("lag_plot_to_file");
        assert!(std::fs::read_to_string(&lag_file).expect("read lag").contains("<svg"));

        // 2. Series autocorrelation_plot
        let ac_spec = sx.autocorrelation_plot().expect("autocorrelation_plot");
        assert_eq!(ac_spec.kind, PlotKind::Line);
        assert_eq!(ac_spec.series.len(), 1);
        let ac_svg = sx.autocorrelation_plot_to_svg().expect("autocorrelation_plot_to_svg");
        assert!(ac_svg.contains("<svg"));
        assert!(ac_svg.contains("<polyline"));
        let ac_html = sx.autocorrelation_plot_to_html().expect("autocorrelation_plot_to_html");
        assert!(ac_html.contains("<div class=\"frankenpandas-plot\""));
        let ac_file = temp_dir.join("autocorr.svg");
        sx.autocorrelation_plot_to_file(&ac_file).expect("autocorrelation_plot_to_file");
        assert!(std::fs::read_to_string(&ac_file).expect("read autocorr").contains("<svg"));

        // 3. Error cases for lag_plot and autocorrelation_plot
        assert!(sg.lag_plot(1).is_err());
        assert!(sg.autocorrelation_plot().is_err());
        assert!(sx.lag_plot(10).is_err());
        let s_short = Series::from_values("short", vec![labels[0].clone()], floats(&[1.0])).unwrap();
        assert!(s_short.autocorrelation_plot().is_err());

        // 4. DataFrame hist_columns_by and hist_by_all
        let hc_by_svg = df.hist_columns_by_to_svg(&["val", "val2"], "group", 5).expect("hist_columns_by_to_svg");
        assert!(hc_by_svg.contains("<svg"));
        let hc_by_html = df.hist_columns_by_to_html(&["val", "val2"], "group", 5).expect("hist_columns_by_to_html");
        assert!(hc_by_html.contains("<div class=\"frankenpandas-plot\""));
        let hc_by_file = temp_dir.join("hist_cols_by.svg");
        df.hist_columns_by_to_file(&["val", "val2"], "group", 5, &hc_by_file).expect("hist_columns_by_to_file");
        assert!(std::fs::read_to_string(&hc_by_file).expect("read hist_cols_by").contains("<svg"));

        let h_all_svg = df.hist_by_all_to_svg("group", 4).expect("hist_by_all_to_svg");
        assert!(h_all_svg.contains("<svg"));
        let h_all_html = df.hist_by_all_to_html("group", 4).expect("hist_by_all_to_html");
        assert!(h_all_html.contains("<div class=\"frankenpandas-plot\""));
        let h_all_file = temp_dir.join("hist_by_all.svg");
        df.hist_by_all_to_file("group", 4, &h_all_file).expect("hist_by_all_to_file");
        assert!(std::fs::read_to_string(&h_all_file).expect("read hist_by_all").contains("<svg"));

        // 5. DataFrame boxplot_columns_by and boxplot_by_all
        let bc_by_svg = df.boxplot_columns_by_to_svg(&["val", "val2"], "group").expect("boxplot_columns_by_to_svg");
        assert!(bc_by_svg.contains("<svg"));
        assert!(bc_by_svg.contains("<line"));
        let bc_by_html = df.boxplot_columns_by_to_html(&["val", "val2"], "group").expect("boxplot_columns_by_to_html");
        assert!(bc_by_html.contains("<div class=\"frankenpandas-plot\""));
        let bc_by_file = temp_dir.join("box_cols_by.svg");
        df.boxplot_columns_by_to_file(&["val", "val2"], "group", &bc_by_file).expect("boxplot_columns_by_to_file");
        assert!(std::fs::read_to_string(&bc_by_file).expect("read box_cols_by").contains("<svg"));

        let b_all_svg = df.boxplot_by_all_to_svg("group").expect("boxplot_by_all_to_svg");
        assert!(b_all_svg.contains("<svg"));
        let b_all_html = df.boxplot_by_all_to_html("group").expect("boxplot_by_all_to_html");
        assert!(b_all_html.contains("<div class=\"frankenpandas-plot\""));
        let b_all_file = temp_dir.join("box_by_all.svg");
        df.boxplot_by_all_to_file("group", &b_all_file).expect("boxplot_by_all_to_file");
        assert!(std::fs::read_to_string(&b_all_file).expect("read box_by_all").contains("<svg"));

        // 6. Error handling
        assert!(df.hist_columns_by(&["nonexistent"], "group", 5).is_err());
        assert!(df.hist_columns_by(&["val"], "nonexistent", 5).is_err());
        assert!(df.boxplot_columns_by(&["nonexistent"], "group").is_err());
        assert!(df.boxplot_columns_by(&["val"], "nonexistent").is_err());
        let df_str_only = DataFrame::from_series(vec![sg.clone()]).unwrap();
        assert!(df_str_only.hist_by_all("group", 5).is_err());
        assert!(df_str_only.boxplot_by_all("group").is_err());

        // Cleanup
        let _ = std::fs::remove_file(&lag_file);
        let _ = std::fs::remove_file(&ac_file);
        let _ = std::fs::remove_file(&hc_by_file);
        let _ = std::fs::remove_file(&h_all_file);
        let _ = std::fs::remove_file(&bc_by_file);
        let _ = std::fs::remove_file(&b_all_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    fn bootstrap_plot_and_multivariate_plots() {
        let labels: Vec<IndexLabel> = (0..6).map(IndexLabel::Int64).collect();
        let sx = Series::from_values(
            "sepal_len",
            labels.clone(),
            floats(&[5.1, 4.9, 4.7, 7.0, 6.4, 6.9]),
        )
        .unwrap();
        let sy = Series::from_values(
            "sepal_wid",
            labels.clone(),
            floats(&[3.5, 3.0, 3.2, 3.2, 3.2, 3.1]),
        )
        .unwrap();
        let sz = Series::from_values(
            "petal_len",
            labels.clone(),
            floats(&[1.4, 1.4, 1.3, 4.7, 4.5, 4.9]),
        )
        .unwrap();
        let sc = Series::from_values(
            "species",
            labels.clone(),
            vec![
                Scalar::Utf8("setosa".into()),
                Scalar::Utf8("setosa".into()),
                Scalar::Utf8("setosa".into()),
                Scalar::Utf8("versicolor".into()),
                Scalar::Utf8("versicolor".into()),
                Scalar::Utf8("versicolor".into()),
            ],
        )
        .unwrap();

        let df =
            DataFrame::from_series(vec![sx.clone(), sy.clone(), sz.clone(), sc.clone()]).unwrap();
        let temp_dir =
            std::env::temp_dir().join(format!("fp_test_multivariate_plots_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. Series bootstrap_plot
        let boot_spec = sx.bootstrap_plot(4, 50).expect("bootstrap_plot");
        assert_eq!(boot_spec.series.len(), 1);
        assert_eq!(boot_spec.series[0].values.len(), 50);
        let boot_svg = sx
            .bootstrap_plot_to_svg(4, 50)
            .expect("bootstrap_plot_to_svg");
        assert!(boot_svg.contains("<svg"));
        assert!(boot_svg.contains("<rect"));
        let boot_html = sx
            .bootstrap_plot_to_html(4, 50)
            .expect("bootstrap_plot_to_html");
        assert!(boot_html.contains("<div class=\"frankenpandas-plot\""));
        let boot_file = temp_dir.join("boot.svg");
        sx.bootstrap_plot_to_file(4, 50, &boot_file)
            .expect("bootstrap_plot_to_file");
        assert!(std::fs::read_to_string(&boot_file)
            .expect("read boot")
            .contains("<svg"));

        // 2. DataFrame parallel_coordinates
        let pc_spec = df
            .parallel_coordinates("species", None)
            .expect("parallel_coordinates");
        assert_eq!(pc_spec.kind, PlotKind::Line);
        assert_eq!(pc_spec.series.len(), 6);
        let pc_svg = df
            .parallel_coordinates_to_svg("species", None)
            .expect("parallel_coordinates_to_svg");
        assert!(pc_svg.contains("<svg"));
        assert!(pc_svg.contains("stroke-dasharray=\"3,3\""));
        assert!(pc_svg.contains("stroke-opacity=\"0.6\""));
        assert!(pc_svg.contains("setosa"));
        assert!(pc_svg.contains("versicolor"));
        let pc_html = df
            .parallel_coordinates_to_html("species", None)
            .expect("parallel_coordinates_to_html");
        assert!(pc_html.contains("<div class=\"frankenpandas-plot\""));
        let pc_file = temp_dir.join("parallel.svg");
        df.parallel_coordinates_to_file("species", None, &pc_file)
            .expect("parallel_coordinates_to_file");
        assert!(std::fs::read_to_string(&pc_file)
            .expect("read parallel")
            .contains("<svg"));

        // parallel_coordinates with explicit cols
        let pc_sub_svg = df
            .parallel_coordinates_to_svg("species", Some(&["sepal_len", "sepal_wid"]))
            .expect("pc_sub");
        assert!(pc_sub_svg.contains("<svg"));

        // 3. DataFrame andrews_curves
        let ac_spec = df.andrews_curves("species", 30).expect("andrews_curves");
        assert_eq!(ac_spec.kind, PlotKind::Line);
        assert_eq!(ac_spec.series.len(), 6);
        assert_eq!(ac_spec.series[0].values.len(), 30);
        let ac_svg = df
            .andrews_curves_to_svg("species", 30)
            .expect("andrews_curves_to_svg");
        assert!(ac_svg.contains("<svg"));
        assert!(ac_svg.contains("stroke-opacity=\"0.6\""));
        assert!(ac_svg.contains("setosa"));
        assert!(ac_svg.contains("versicolor"));
        let ac_html = df
            .andrews_curves_to_html("species", 30)
            .expect("andrews_curves_to_html");
        assert!(ac_html.contains("<div class=\"frankenpandas-plot\""));
        let ac_file = temp_dir.join("andrews.svg");
        df.andrews_curves_to_file("species", 30, &ac_file)
            .expect("andrews_curves_to_file");
        assert!(std::fs::read_to_string(&ac_file)
            .expect("read andrews")
            .contains("<svg"));

        // 4. DataFrame radviz
        let rv_spec = df.radviz("species", None).expect("radviz");
        assert_eq!(rv_spec.kind, PlotKind::Scatter);
        assert_eq!(rv_spec.series.len(), 4); // 2 classes * 2 (x, y)
        let rv_svg = df.radviz_to_svg("species", None).expect("radviz_to_svg");
        assert!(rv_svg.contains("<svg"));
        assert!(rv_svg.contains("<ellipse")); // circular boundary
        assert!(rv_svg.contains("<circle")); // scatter points
        assert!(rv_svg.contains("setosa"));
        assert!(rv_svg.contains("versicolor"));
        let rv_html = df.radviz_to_html("species", None).expect("radviz_to_html");
        assert!(rv_html.contains("<div class=\"frankenpandas-plot\""));
        let rv_file = temp_dir.join("radviz.svg");
        df.radviz_to_file("species", None, &rv_file)
            .expect("radviz_to_file");
        assert!(std::fs::read_to_string(&rv_file)
            .expect("read radviz")
            .contains("<svg"));

        // 5. Error conditions
        assert!(sc.bootstrap_plot(5, 10).is_err());
        assert!(df.parallel_coordinates("nonexistent", None).is_err());
        assert!(df
            .parallel_coordinates("species", Some(&["nonexistent"]))
            .is_err());
        assert!(df.andrews_curves("nonexistent", 50).is_err());
        assert!(df.radviz("nonexistent", None).is_err());
        assert!(df.radviz("species", Some(&["species"])).is_err()); // no numeric cols
        let df_str = DataFrame::from_series(vec![sc.clone()]).unwrap();
        assert!(df_str.parallel_coordinates("species", None).is_err());
        assert!(df_str.andrews_curves("species", 50).is_err());
        assert!(df_str.radviz("species", None).is_err());

        // Cleanup
        let _ = std::fs::remove_file(&boot_file);
        let _ = std::fs::remove_file(&pc_file);
        let _ = std::fs::remove_file(&ac_file);
        let _ = std::fs::remove_file(&rv_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }
}
