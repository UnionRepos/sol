use crate::syntax::{self, ComponentValue as C, TokenKind as T};
use crate::values::Value;
use std::collections::{HashMap, HashSet};

pub const MAX_EXPANDED_COMPONENTS: usize = 65_536;
const MAX_VARIABLE_DEPTH: usize = 128;

pub(crate) fn contains_var(values: &[C]) -> bool {
    values.iter().any(|v| match v {
        C::Function { name, value } => name.eq_ignore_ascii_case("var") || contains_var(value),
        C::Block { value, .. } => contains_var(value),
        _ => false,
    })
}
pub(crate) fn declaration_value(name: &str, values: &[C]) -> Vec<Value> {
    if name.starts_with("--") || contains_var(values) {
        vec![Value::Components(values.to_vec())]
    } else {
        crate::values::from_components(values)
    }
}
pub(crate) fn deferred(values: &[Value]) -> bool {
    matches!(values, [Value::Components(_)])
}

pub(crate) struct Variables {
    source: HashMap<String, Vec<C>>,
    cache: HashMap<String, Option<Vec<C>>>,
    cyclic: HashSet<String>,
}
impl Variables {
    pub(crate) fn new(source: HashMap<String, Vec<C>>) -> Self {
        let mut cyclic = HashSet::new();
        let mut done = HashSet::new();
        let mut stack = Vec::new();
        // Fallback references participate in the dependency graph even when
        // their branch would not run; accepting those cycles makes results
        // depend on lookup order instead of the computed-value rules.
        for name in source.keys() {
            cycles(name, &source, &mut stack, &mut done, &mut cyclic);
        }
        Self {
            source,
            cache: HashMap::new(),
            cyclic,
        }
    }
    fn property(&mut self, name: &str, depth: usize) -> Option<Vec<C>> {
        if self.cyclic.contains(name) || depth > MAX_VARIABLE_DEPTH {
            return None;
        }
        if let Some(cached) = self.cache.get(name) {
            return cached.clone();
        }
        let source = self.source.get(name)?.clone();
        let value = self.substitute(&source, depth + 1);
        self.cache.insert(name.to_string(), value.clone());
        value
    }
    pub(crate) fn substitute(&mut self, values: &[C], depth: usize) -> Option<Vec<C>> {
        if depth > MAX_VARIABLE_DEPTH {
            return None;
        }
        let mut out = Vec::new();
        let mut expanded = 0;
        for value in values {
            let start = out.len();
            match value {
                C::Function { name, value } if name.eq_ignore_ascii_case("var") => {
                    let comma = value.iter().position(|v| v.kind() == Some(&T::Comma));
                    let [C::Token(token)] = syntax::trim(&value[..comma.unwrap_or(value.len())])
                    else {
                        return None;
                    };
                    let T::Ident(name) = &token.kind else {
                        return None;
                    };
                    if !name.starts_with("--") || name == "--" {
                        return None;
                    }
                    let replacement = self.property(name, depth + 1).or_else(|| {
                        comma.and_then(|i| self.substitute(&value[i + 1..], depth + 1))
                    })?;
                    if count(&replacement) > MAX_EXPANDED_COMPONENTS.saturating_sub(expanded) {
                        return None;
                    }
                    out.extend(replacement);
                }
                C::Function { name, value } => out.push(C::Function {
                    name: name.clone(),
                    value: self.substitute(value, depth + 1)?,
                }),
                C::Block { opening, value } => out.push(C::Block {
                    opening: *opening,
                    value: self.substitute(value, depth + 1)?,
                }),
                value => out.push(value.clone()),
            }
            expanded += count(&out[start..]);
            if expanded > MAX_EXPANDED_COMPONENTS {
                return None;
            }
        }
        Some(out)
    }
    pub(crate) fn computed(&mut self) -> HashMap<String, Vec<Value>> {
        let names: Vec<_> = self.source.keys().cloned().collect();
        let mut out = HashMap::new();
        for name in names {
            if let Some(value) = self.property(&name, 0) {
                out.insert(name, vec![Value::Components(value)]);
            }
        }
        out
    }
}
fn count(values: &[C]) -> usize {
    values
        .iter()
        .map(|v| match v {
            C::Function { value, .. } | C::Block { value, .. } => 1 + count(value),
            _ => 1,
        })
        .sum()
}
fn references(values: &[C], out: &mut Vec<String>) {
    for value in values {
        match value {
            C::Function { name, value } => {
                if name.eq_ignore_ascii_case("var") {
                    if let Some(T::Ident(name)) = syntax::trim(value).first().and_then(C::kind) {
                        out.push(name.clone());
                    }
                }
                references(value, out);
            }
            C::Block { value, .. } => references(value, out),
            _ => {}
        }
    }
}
fn cycles(
    name: &str,
    source: &HashMap<String, Vec<C>>,
    stack: &mut Vec<String>,
    done: &mut HashSet<String>,
    cyclic: &mut HashSet<String>,
) {
    if let Some(start) = stack.iter().position(|n| n == name) {
        cyclic.extend(stack[start..].iter().cloned());
        return;
    }
    if done.contains(name) {
        return;
    }
    if stack.len() >= MAX_VARIABLE_DEPTH {
        cyclic.insert(name.to_string());
        return;
    }
    let Some(value) = source.get(name) else {
        return;
    };
    stack.push(name.to_string());
    let mut dependencies = Vec::new();
    references(value, &mut dependencies);
    for dependency in dependencies {
        cycles(&dependency, source, stack, done, cyclic);
    }
    stack.pop();
    done.insert(name.to_string());
}
