//! Transition targets must survive independently of sampled output. Otherwise a
//! refresh would mistake its own intermediate value for a new target and restart
//! the effect indefinitely; interrupted effects also need their current sample.
use crate::{
    animations::{interpolate, seconds, timing},
    Style, Value,
};
use sol_dom::NodeId;
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub(crate) struct Transitions {
    previous: HashMap<NodeId, HashMap<String, Vec<Value>>>,
    running: HashMap<(NodeId, String), Transition>,
}
struct Transition {
    from: Vec<Value>,
    to: Vec<Value>,
    start: f64,
    duration: f64,
    curve: Vec<Value>,
}
fn initial(name: &str) -> Vec<Value> {
    match name {
        "opacity" => vec![Value::Number(1.0)],
        name if name.ends_with("color") => {
            vec![Value::Color(0, 0, 0, if name == "color" { 255 } else { 0 })]
        }
        "width" | "height" => vec![Value::Ident("auto".into())],
        _ => vec![Value::Number(0.0)],
    }
}
fn compatible(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| match (a, b) {
            (Value::Number(_), Value::Number(_))
            | (Value::Percentage(_), Value::Percentage(_))
            | (Value::Color(..), Value::Color(..))
            | (Value::Length(..), Value::Length(..)) => true,
            (Value::Func(a, av), Value::Func(b, bv)) => a == b && compatible(av, bv),
            _ => a == b,
        })
}
fn list<'a>(style: &'a Style, name: &str) -> Vec<&'a [Value]> {
    style
        .get(name)
        .map(|v| v.split(|v| matches!(v,Value::Ident(s) if s==",")).collect())
        .unwrap_or_default()
}
fn item<'a>(list: &[&'a [Value]], index: usize) -> &'a [Value] {
    if list.is_empty() {
        &[]
    } else {
        list[index % list.len()]
    }
}
pub(crate) fn expand(values: &[Value]) -> Vec<(String, Vec<Value>)> {
    let names = ["property", "duration", "timing-function", "delay"];
    let mut out: Vec<Vec<Value>> = vec![Vec::new(); 4];
    for (index, group) in values
        .split(|v| matches!(v,Value::Ident(s) if s==","))
        .enumerate()
    {
        let mut properties = vec![
            vec![Value::Ident("all".into())],
            vec![Value::Length(0.0, crate::values::Unit::S)],
            vec![Value::Ident("ease".into())],
            vec![Value::Length(0.0, crate::values::Unit::S)],
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
                Value::Func(name, _) if name == "steps" || name == "cubic-bezier" => 2,
                Value::Ident(name)
                    if matches!(
                        name.as_str(),
                        "linear"
                            | "ease"
                            | "ease-in"
                            | "ease-out"
                            | "ease-in-out"
                            | "step-start"
                            | "step-end"
                    ) =>
                {
                    2
                }
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
        .map(|(name, values)| (format!("transition-{name}"), values))
        .collect()
}
impl Transition {
    fn sample(&self, time: f64) -> Vec<Value> {
        let progress = ((time - self.start) / self.duration).clamp(0.0, 1.0);
        interpolate(&self.from, &self.to, timing(&self.curve, progress))
    }
}
impl Transitions {
    pub(crate) fn retain(&mut self, styles: &HashMap<NodeId, std::rc::Rc<Style>>) {
        self.previous.retain(|node, _| styles.contains_key(node));
        self.running
            .retain(|(node, _), _| styles.contains_key(node));
    }
    pub(crate) fn apply(
        &mut self,
        node: NodeId,
        base: &Style,
        output: &mut Style,
        time: f64,
    ) -> bool {
        let mut active = false;
        let names = list(base, "transition-property");
        let durations = list(base, "transition-duration");
        let delays = list(base, "transition-delay");
        let curves = list(base, "transition-timing-function");
        let mut eligible = HashSet::new();
        if let Some(previous) = self.previous.get(&node) {
            let default = [Value::Ident("all".into())];
            let names = if names.is_empty() {
                vec![default.as_slice()]
            } else {
                names
            };
            for (index, name) in names.iter().enumerate().take(128) {
                let Some(Value::Ident(name)) = name.first() else {
                    continue;
                };
                if name == "none" {
                    continue;
                }
                let properties: Vec<_> = if name == "all" {
                    base.map
                        .keys()
                        .chain(previous.keys())
                        .filter(|name| {
                            !name.starts_with("animation-")
                                && !name.starts_with("transition-")
                                && !name.starts_with("--")
                        })
                        .cloned()
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect()
                } else {
                    vec![name.clone()]
                };
                for property in properties {
                    let duration = seconds(item(&durations, index));
                    let delay = seconds(item(&delays, index));
                    if duration <= 0.0 || duration + delay <= 0.0 {
                        continue;
                    }
                    eligible.insert(property.clone());
                    let to = base
                        .get(&property)
                        .cloned()
                        .unwrap_or_else(|| initial(&property));
                    let old = previous
                        .get(&property)
                        .cloned()
                        .unwrap_or_else(|| initial(&property));
                    let key = (node, property.clone());
                    if to != old {
                        let from = self
                            .running
                            .get(&key)
                            .map(|transition| transition.sample(time))
                            .unwrap_or(old);
                        if compatible(&from, &to) {
                            self.running.insert(
                                key.clone(),
                                Transition {
                                    from,
                                    to,
                                    start: time + delay,
                                    duration,
                                    curve: item(&curves, index).to_vec(),
                                },
                            );
                        } else {
                            self.running.remove(&key);
                        }
                    }
                    if let Some(transition) = self.running.get(&key) {
                        if time < transition.start + transition.duration {
                            output.set(&property, transition.sample(time));
                            active = true;
                        } else {
                            self.running.remove(&key);
                        }
                    }
                }
            }
        }
        self.running
            .retain(|(candidate, property), _| *candidate != node || eligible.contains(property));
        self.previous.insert(node, base.map.clone());
        active
    }
}
