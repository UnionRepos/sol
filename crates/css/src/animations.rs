//! Time belongs to the page rather than individual style passes: container-query
//! feedback can restyle several times per frame, and advancing during those passes
//! would make layout depend on how quickly a machine completes the refresh.
use crate::{
    parser::{KeyframesRule, Rule, Stylesheet},
    style::{expand_decl, MatchedDecl},
    Style, Value,
};
use sol_dom::{Document, NodeId};
use std::collections::HashMap;

#[derive(Default)]
pub struct Timeline {
    pub time: f64,
    pub active: bool,
    starts: HashMap<NodeId, (String, f64)>,
    pub(crate) transitions: crate::transitions::Transitions,
}
fn comma(value: &Value) -> bool {
    matches!(value,Value::Ident(s) if s==",")
}
fn list<'a>(style: &'a Style, name: &str) -> Vec<&'a [Value]> {
    style
        .get(name)
        .map(|v| v.split(comma).collect())
        .unwrap_or_default()
}
fn item<'a>(style: &'a Style, name: &str, index: usize) -> &'a [Value] {
    let list = list(style, name);
    if list.is_empty() {
        &[]
    } else {
        list[index % list.len()]
    }
}
fn ident(value: &[Value], default: &str) -> String {
    value
        .first()
        .and_then(|v| {
            if let Value::Ident(s) | Value::Str(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| default.into())
}
pub(crate) fn seconds(values: &[Value]) -> f64 {
    match values.first() {
        Some(Value::Length(n, crate::values::Unit::S)) => *n,
        Some(Value::Length(n, crate::values::Unit::Ms)) => n / 1000.0,
        _ => 0.0,
    }
}
pub(crate) fn expand(values: &[Value]) -> Vec<(String, Vec<Value>)> {
    let names = [
        "name",
        "duration",
        "timing-function",
        "delay",
        "iteration-count",
        "direction",
        "fill-mode",
        "play-state",
    ];
    let mut out: Vec<Vec<Value>> = vec![Vec::new(); names.len()];
    for (index, group) in values.split(comma).enumerate() {
        let mut properties = vec![
            vec![Value::Ident("none".into())],
            vec![Value::Length(0.0, crate::values::Unit::S)],
            vec![Value::Ident("ease".into())],
            vec![Value::Length(0.0, crate::values::Unit::S)],
            vec![Value::Number(1.0)],
            vec![Value::Ident("normal".into())],
            vec![Value::Ident("none".into())],
            vec![Value::Ident("running".into())],
        ];
        let mut duration = false;
        for value in group {
            let slot = match value {
                Value::Length(_, crate::values::Unit::S | crate::values::Unit::Ms) => {
                    if duration {
                        3
                    } else {
                        duration = true;
                        1
                    }
                }
                Value::Number(_) => 4,
                Value::Func(name, _) if name == "steps" || name == "cubic-bezier" => 2,
                Value::Ident(name) => match name.as_str() {
                    "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" | "step-start"
                    | "step-end" => 2,
                    "infinite" => 4,
                    "normal" | "reverse" | "alternate" | "alternate-reverse" => 5,
                    "forwards" | "backwards" | "both" => 6,
                    "running" | "paused" => 7,
                    _ => 0,
                },
                _ => 0,
            };
            properties[slot] = vec![value.clone()];
        }
        for (target, property) in out.iter_mut().zip(properties) {
            if index > 0 {
                target.push(Value::Ident(",".into()));
            }
            target.extend(property);
        }
    }
    names
        .into_iter()
        .zip(out)
        .map(|(name, values)| (format!("animation-{name}"), values))
        .collect()
}
pub(crate) fn registry<'a>(sheets: &[&'a Stylesheet]) -> HashMap<String, &'a KeyframesRule> {
    fn collect<'a>(rules: &'a [Rule], out: &mut HashMap<String, &'a KeyframesRule>) {
        for rule in rules {
            match rule {
                Rule::Keyframes(frames) => {
                    out.insert(frames.name.clone(), frames);
                }
                Rule::Media(rule) if crate::conditions::media_matches(&rule.queries) => {
                    collect(&rule.rules, out)
                }
                Rule::Supports(rule) if rule.condition.supports() => collect(&rule.rules, out),
                Rule::Layer(rule) => collect(&rule.rules, out),
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for sheet in sheets {
        collect(&sheet.rules, &mut out);
    }
    out
}
pub(crate) fn timing(values: &[Value], progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let points = match values.first() {
        Some(Value::Ident(name)) => match name.as_str() {
            "linear" => return progress,
            "ease-in" => [0.42, 0.0, 1.0, 1.0],
            "ease-out" => [0.0, 0.0, 0.58, 1.0],
            "ease-in-out" => [0.42, 0.0, 0.58, 1.0],
            "step-start" => return 1.0,
            "step-end" => return f64::from(progress == 1.0),
            _ => [0.25, 0.1, 0.25, 1.0],
        },
        Some(Value::Func(name, args)) if name == "cubic-bezier" => {
            let values: Vec<_> = args
                .iter()
                .filter_map(|v| {
                    if let Value::Number(n) = v {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .collect();
            let [x1, y1, x2, y2] = values.as_slice() else {
                return progress;
            };
            if !(0.0..=1.0).contains(x1) || !(0.0..=1.0).contains(x2) {
                return progress;
            }
            [*x1, *y1, *x2, *y2]
        }
        Some(Value::Func(name, args)) if name == "steps" => {
            let Some(Value::Number(count)) = args.first() else {
                return progress;
            };
            if *count < 1.0 || count.fract() != 0.0 {
                return progress;
            }
            let jump = args
                .iter()
                .find_map(|v| {
                    if let Value::Ident(s) = v {
                        (s != ",").then_some(s.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("end");
            let offset = f64::from(matches!(jump, "start" | "jump-start" | "jump-both"));
            let denominator = *count
                + if jump == "jump-both" {
                    1.0
                } else if jump == "jump-none" {
                    -1.0
                } else {
                    0.0
                };
            return ((progress * count).floor() + offset).clamp(0.0, denominator)
                / denominator.max(1.0);
        }
        _ => [0.25, 0.1, 0.25, 1.0],
    };
    let curve = |t: f64, a: f64, b: f64| {
        3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t.powi(3)
    };
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if curve(mid, points[0], points[2]) < progress {
            lo = mid
        } else {
            hi = mid
        }
    }
    curve((lo + hi) / 2.0, points[1], points[3])
}
pub(crate) fn interpolate(from: &[Value], to: &[Value], t: f64) -> Vec<Value> {
    if from.len() != to.len() {
        return if t < 0.5 { from.to_vec() } else { to.to_vec() };
    }
    from.iter()
        .zip(to)
        .map(|(a, b)| match (a, b) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a + (b - a) * t),
            (Value::Percentage(a), Value::Percentage(b)) => Value::Percentage(a + (b - a) * t),
            (Value::Length(a, u), Value::Length(b, v)) if u == v => {
                Value::Length(a + (b - a) * t, *u)
            }
            (Value::Length(..), Value::Length(..)) => match (
                crate::style::value_px(a, 16.0, 800.0),
                crate::style::value_px(b, 16.0, 800.0),
            ) {
                (Some(a), Some(b)) => Value::Length(a + (b - a) * t, crate::values::Unit::Px),
                _ => {
                    if t < 0.5 {
                        a.clone()
                    } else {
                        b.clone()
                    }
                }
            },
            (Value::Color(ar, ag, ab, aa), Value::Color(br, bg, bb, ba)) => {
                let alpha = *aa as f64 * (1.0 - t) + *ba as f64 * t;
                let c = |a: u8, b: u8| {
                    if alpha <= 0.0 {
                        0
                    } else {
                        ((a as f64 * *aa as f64 * (1.0 - t) + b as f64 * *ba as f64 * t) / alpha)
                            .round()
                            .clamp(0.0, 255.0) as u8
                    }
                };
                Value::Color(
                    c(*ar, *br),
                    c(*ag, *bg),
                    c(*ab, *bb),
                    alpha.round().clamp(0.0, 255.0) as u8,
                )
            }
            (Value::Func(an, a), Value::Func(bn, b)) if an == bn => {
                Value::Func(an.clone(), interpolate(a, b, t))
            }
            _ => {
                if t < 0.5 {
                    a.clone()
                } else {
                    b.clone()
                }
            }
        })
        .collect()
}
impl Timeline {
    pub(crate) fn retain(&mut self, styles: &HashMap<NodeId, std::rc::Rc<Style>>) {
        self.starts.retain(|node, _| styles.contains_key(node));
        self.transitions.retain(styles);
    }
    pub(crate) fn apply(
        &mut self,
        doc: &Document,
        node: NodeId,
        style: &mut Style,
        frames: &HashMap<String, &KeyframesRule>,
        important: &std::collections::HashSet<String>,
    ) {
        let names = list(style, "animation-name");
        if names.is_empty() {
            self.starts.remove(&node);
            return;
        }
        let signature = format!(
            "{:?}",
            style
                .map
                .iter()
                .filter(|(name, _)| name.starts_with("animation-"))
                .collect::<std::collections::BTreeMap<_, _>>()
        );
        let start = self
            .starts
            .entry(node)
            .or_insert_with(|| (signature.clone(), self.time));
        if start.0 != signature {
            *start = (signature, self.time);
        }
        let elapsed = (self.time - start.1).max(0.0);
        let underlying = style.clone();
        let animations: Vec<_> = names
            .into_iter()
            .take(128)
            .map(|values| ident(values, "none"))
            .collect();
        for (index, name) in animations.into_iter().enumerate() {
            let Some(frames) = frames.get(&name) else {
                continue;
            };
            let duration = seconds(item(&underlying, "animation-duration", index)).max(0.0);
            let delay = seconds(item(&underlying, "animation-delay", index));
            let iterations = match item(&underlying, "animation-iteration-count", index).first() {
                Some(Value::Number(n)) => n.max(0.0),
                Some(Value::Ident(s)) if s == "infinite" => f64::INFINITY,
                _ => 1.0,
            };
            let paused =
                ident(item(&underlying, "animation-play-state", index), "running") == "paused";
            let local = if paused { 0.0 } else { elapsed } - delay;
            let end = duration * iterations;
            let fill = ident(item(&underlying, "animation-fill-mode", index), "none");
            if !paused && local < end {
                self.active = true;
            }
            if local < 0.0 && !matches!(fill.as_str(), "backwards" | "both")
                || local >= end && !matches!(fill.as_str(), "forwards" | "both")
            {
                continue;
            }
            let finished = local >= end;
            let (iteration, mut progress) = if local < 0.0 {
                (0.0, 0.0)
            } else if finished {
                if iterations.fract() == 0.0 {
                    ((iterations - 1.0).max(0.0), 1.0)
                } else {
                    (iterations.floor(), iterations.fract())
                }
            } else {
                let p = local / duration;
                (p.floor(), p.fract())
            };
            let direction = ident(item(&underlying, "animation-direction", index), "normal");
            if direction == "reverse"
                || direction == "alternate" && iteration % 2.0 == 1.0
                || direction == "alternate-reverse" && iteration % 2.0 == 0.0
            {
                progress = 1.0 - progress;
            }
            let mut properties: HashMap<String, Vec<(f64, Vec<Value>)>> = HashMap::new();
            for frame in &frames.frames {
                let matched = frame
                    .decls
                    .iter()
                    .enumerate()
                    .flat_map(|(order, decl)| {
                        expand_decl(&decl.name, decl.value.clone()).into_iter().map(
                            move |(name, value)| MatchedDecl {
                                name,
                                value,
                                important: false,
                                specificity: (0, 0, 0),
                                origin: 1,
                                layer: usize::MAX,
                                order,
                            },
                        )
                    })
                    .collect();
                let computed = crate::compute(doc, node, matched, Some(&underlying));
                for (name, value) in computed.map {
                    if name.starts_with("animation-")
                        || important.contains(&name)
                        || !frame.decls.iter().any(|decl| {
                            name == decl.name || name.starts_with(&format!("{}-", decl.name))
                        })
                    {
                        continue;
                    }
                    properties
                        .entry(name)
                        .or_default()
                        .push((frame.offset, value));
                }
            }
            for (property, mut values) in properties {
                let base =
                    underlying
                        .get(&property)
                        .cloned()
                        .unwrap_or_else(|| match property.as_str() {
                            "opacity" => vec![Value::Number(1.0)],
                            name if name.ends_with("color") => {
                                vec![Value::Color(0, 0, 0, if name == "color" { 255 } else { 0 })]
                            }
                            name if name == "width" || name == "height" => {
                                vec![Value::Ident("auto".into())]
                            }
                            _ => vec![Value::Number(0.0)],
                        });
                if values.first().is_none_or(|v| v.0 != 0.0) {
                    values.insert(0, (0.0, base.clone()));
                }
                if values.last().is_none_or(|v| v.0 != 1.0) {
                    values.push((1.0, base));
                }
                let right = values
                    .partition_point(|(offset, _)| *offset <= progress)
                    .min(values.len() - 1);
                let left = right.saturating_sub(1);
                let t = if values[right].0 == values[left].0 {
                    1.0
                } else {
                    (progress - values[left].0) / (values[right].0 - values[left].0)
                };
                let t = timing(item(&underlying, "animation-timing-function", index), t);
                style.set(&property, interpolate(&values[left].1, &values[right].1, t));
            }
        }
    }
}
