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
            Scalar::Float64(v) => Ok(Some(*v)),
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

fn legend(entries: &[(String, &str)]) -> String {
    let mut out = String::new();
    let y = HEIGHT - 10.0;
    for (i, (name, color)) in entries.iter().enumerate() {
        let x = MARGIN_LEFT + 8.0 + (i as f64 * 150.0);
        out.push_str(&format!(
            "<rect x=\"{x}\" y=\"{:.2}\" width=\"10\" height=\"10\" fill=\"{color}\"/><text x=\"{}\" y=\"{y}\" fill=\"#333\">{}</text>",
            y - 9.0,
            x + 14.0,
            esc(name)
        ));
    }
    out
}

// ── PlotSpec rendering (line / area / scatter / bar / pie) ─────────────────

fn render_plot(spec: &PlotSpec) -> Result<String, FrameError> {
    let views: Result<Vec<Vec<Option<f64>>>, FrameError> =
        spec.series.iter().map(numeric_view).collect();
    let views = views?;
    let scale = data_scale(&views)?;
    let n = spec.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    let x_labels: Vec<String> = spec
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

    let y = |v: f64| MARGIN_TOP + (scale.max - v) / (scale.max - scale.min) * PLOT_H;
    let x = |i: usize| MARGIN_LEFT + (i as f64 + 0.5) * PLOT_W / n.max(1) as f64;

    let mut body = String::new();
    match spec.kind {
        PlotKind::Line | PlotKind::Area => {
            // Contiguous finite runs become separate polylines; NaN/Null breaks
            // the line exactly like a masked gap.
            for (si, values) in views.iter().enumerate() {
                let color = palette(si);
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
                        "<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\"/>"
                    ));
                }
            }
        }
        PlotKind::Scatter => {
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
        PlotKind::Bar => {
            let k = views.len().max(1);
            let slot = PLOT_W / n.max(1) as f64;
            let bar_w = slot * 0.7 / k as f64;
            let base = y(scale.min);
            for (si, values) in views.iter().enumerate() {
                let color = palette(si);
                let offset = si as f64 * bar_w + slot * 0.15;
                for (i, v) in values.iter().enumerate() {
                    if let Some(v) = v.filter(|v| v.is_finite()) {
                        let top = y(v);
                        body.push_str(&format!(
                            "<rect x=\"{:.2}\" y=\"{top:.2}\" width=\"{bar_w:.2}\" height=\"{:.2}\" fill=\"{color}\"/>",
                            MARGIN_LEFT + i as f64 * slot + offset,
                            (base - top).abs().max(0.5)
                        ));
                    }
                }
            }
        }
        PlotKind::Pie => {
            // Wedges from each series' share of its sum; negative totals are a
            // fail-closed error (pandas raises too); NaN skipped.
            let mut angle: f64 = -std::f64::consts::FRAC_PI_2;
            let cx = MARGIN_LEFT + PLOT_W / 2.0;
            let cy = MARGIN_TOP + PLOT_H / 2.0;
            let r = 140.0;
            for (si, values) in views.iter().enumerate() {
                let total: f64 = values.iter().flatten().copied().sum();
                if values.iter().flatten().any(|v| *v < 0.0) {
                    return Err(FrameError::CompatibilityRejected(format!(
                        "pie requires non-negative values: series '{}' has a negative slice",
                        spec.series[si].name
                    )));
                }
                if total == 0.0 {
                    continue;
                }
                let color = palette(si);
                for v in values.iter().flatten().copied() {
                    let next = angle + (v / total) * std::f64::consts::TAU;
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
        // The plot() hooks never emit these two, but be total rather than
        // failing mysteriously: route through the histogram primitives.
        PlotKind::Histogram | PlotKind::Box => {
            let finite: Vec<Vec<f64>> = views
                .iter()
                .map(|v| v.iter().flatten().copied().collect())
                .collect();
            let names: Vec<String> = spec.series.iter().map(|s| s.name.clone()).collect();
            body = histogram_body(&finite, &names, 10, &format!("{} (as bars)", spec.method))?;
        }
    }

    let legend_entries: Vec<(String, &str)> = spec
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.clone(), palette(i)))
        .collect();
    Ok(format!(
        "{}{body}{}{}</svg>",
        svg_open(&spec.method),
        svg_axes(&scale, &x_labels),
        legend(&legend_entries),
    ))
}

// ── Histogram primitives ───────────────────────────────────────────────────

/// Bar body for binned counts: one inset of `bins` bars per series, laid out
/// side by side. `scale` spans ALL series so heights are comparable.
fn histogram_body(
    series: &[Vec<f64>],
    _names: &[String],
    bins: usize,
    title: &str,
) -> Result<String, FrameError> {
    if bins == 0 {
        return Err(FrameError::CompatibilityRejected(
            "histogram requires at least one bin".to_owned(),
        ));
    }
    let flat: Vec<Option<f64>> = series.iter().flat_map(|v| v.iter().copied()).map(Some).collect();
    let scale = data_scale(&[flat])?;
    let step = (scale.max - scale.min) / bins as f64;

    let mut global_max = 0usize;
    let mut all_counts: Vec<Vec<usize>> = Vec::with_capacity(series.len());
    for values in series {
        let mut counts = vec![0usize; bins];
        for v in values {
            // Right-closed last bin, matching pandas' `np.histogram` default.
            let idx = if *v == scale.max {
                bins - 1
            } else {
                (((v - scale.min) / step).floor() as usize).min(bins - 1)
            };
            counts[idx] += 1;
        }
        global_max = global_max.max(*counts.iter().max().unwrap_or(&0));
        all_counts.push(counts);
    }
    let global_max = global_max.max(1);

    let mut body = String::new();
    let width = PLOT_W / series.len().max(1) as f64;
    for (si, counts) in all_counts.iter().enumerate() {
        let color = palette(si);
        let x0 = MARGIN_LEFT + si as f64 * width;
        let inner = width * 0.85;
        let bar_w = inner / bins as f64 - 1.0;
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
    Ok(format!(
        "{}{body}</svg>",
        svg_open(title)
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
}

impl HistogramSpec {
    /// Render a grouped histogram: one inset of `bins` bars per series, NaN
    /// values skipped. Fails closed on non-numeric input or all-missing data.
    pub fn to_svg(&self) -> Result<String, FrameError> {
        let views: Result<Vec<Vec<Option<f64>>>, FrameError> =
            self.series.iter().map(numeric_view).collect();
        let views = views?;
        let finite: Vec<Vec<f64>> = views
            .iter()
            .map(|v| v.iter().flatten().copied().collect())
            .collect();
        if finite.iter().all(|v| v.is_empty()) {
            return Err(FrameError::CompatibilityRejected(
                "no plottable (non-missing) values in this histogram".to_owned(),
            ));
        }
        let names: Vec<String> = self.series.iter().map(|s| s.name.clone()).collect();
        let title = format!("histogram ({})", names.first().map(String::as_str).unwrap_or("values"));
        histogram_body(&finite, &names, self.bins, &title)
    }

    /// [`HistogramSpec::to_svg`] as UTF-8 bytes.
    pub fn to_svg_bytes(&self) -> Result<Vec<u8>, FrameError> {
        self.to_svg().map(String::into_bytes)
    }
}

impl BoxPlotSpec {
    /// Render a side-by-side box-and-whisker plot (min, q1, median, q3, max —
    /// linear-interpolated quantiles, NaN skipped). Fails closed when every
    /// series is empty, or on non-numeric input.
    pub fn to_svg(&self) -> Result<String, FrameError> {
        let views: Result<Vec<Vec<Option<f64>>>, FrameError> =
            self.series.iter().map(numeric_view).collect();
        let views = views?;
        let mut sorted: Vec<Vec<f64>> = views
            .iter()
            .map(|v| {
                let mut vals: Vec<f64> = v.iter().flatten().copied().collect();
                vals.sort_by(f64::total_cmp);
                vals
            })
            .collect();
        sorted.retain(|vals| !vals.is_empty());
        if sorted.is_empty() {
            return Err(FrameError::CompatibilityRejected(
                "no plottable (non-missing) values in this boxplot".to_owned(),
            ));
        }
        let scale = data_scale(&views)?;
        let y = |v: f64| MARGIN_TOP + (scale.max - v) / (scale.max - scale.min) * PLOT_H;
        let slot = PLOT_W / sorted.len() as f64;
        let mut body = String::new();
        for (si, vals) in sorted.iter().enumerate() {
            let color = palette(si);
            let (q1, med, q3) = (quantile(vals, 0.25), quantile(vals, 0.5), quantile(vals, 0.75));
            let (lo, hi) = (vals[0], vals[vals.len() - 1]);
            let cx = MARGIN_LEFT + slot * (si as f64 + 0.5);
            let bw = slot * 0.5;
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
        let title = format!(
            "boxplot ({})",
            self.series
                .first()
                .map(|s| s.name.as_str())
                .unwrap_or("values")
        );
        Ok(format!("{}{body}</svg>", svg_open(&title)))
    }

    /// [`BoxPlotSpec::to_svg`] as UTF-8 bytes.
    pub fn to_svg_bytes(&self) -> Result<Vec<u8>, FrameError> {
        self.to_svg().map(String::into_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        numeric_view, quantile, BoxPlotSpec, FrameError, HistogramSpec, PlotKind, PlotSeriesSpec,
        PlotSpec, Scalar,
    };
    use crate::DType;

    fn series(name: &str, values: Vec<Scalar>) -> PlotSeriesSpec {
        PlotSeriesSpec {
            name: name.to_owned(),
            dtype: DType::Float64,
            index: (0..values.len() as i64).map(crate::IndexLabel::Int64).collect(),
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
        assert!(!svg.contains("NaN"), "missing values must not leak as text coords");
    }

    #[test]
    fn all_missing_and_non_numeric_series_fail_closed() {
        let all_nan = PlotSpec {
            method: "plot".to_owned(),
            kind: PlotKind::Line,
            series: vec![series("nan", vec![Scalar::Float64(f64::NAN); 3])],
        };
        assert!(matches!(all_nan.to_svg(), Err(FrameError::CompatibilityRejected(_))));

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
        assert!(!svg.contains("<b>&"), "raw XML-sensitive text must not leak");
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
        assert!(matches!(negative_pie.to_svg(), Err(FrameError::CompatibilityRejected(_))));
    }
}
