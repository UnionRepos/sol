//! Import expansion and URL resolution share a source budget because recursive
//! CSS must not turn one bounded response into an unbounded retained sheet tree.
use crate::Page;
use sol_css::syntax::{self, ComponentValue as C, Rule, TokenKind as T};
use sol_url::Url;
use std::collections::HashSet;
const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Default)]
pub(super) struct Budget {
    bytes: usize,
    imports: usize,
}
fn resolve(values: &mut [C], base: &Url) {
    for value in values {
        match value {
            C::Token(token) => {
                if let T::Url(url) = &mut token.kind {
                    if let Ok(resolved) = base.join(url) {
                        *url = resolved.serialize();
                    }
                }
            }
            C::Function { name, value } if name.eq_ignore_ascii_case("url") => {
                if let [C::Token(token)] = syntax::trim(value) {
                    if let T::String(url) = &token.kind {
                        if let Ok(url) = base.join(url) {
                            let mut token = token.clone();
                            token.kind = T::String(url.serialize());
                            *value = vec![C::Token(token)];
                        }
                    }
                }
            }
            C::Function { value, .. } | C::Block { value, .. } => resolve(value, base),
        }
    }
}
impl Page {
    pub(super) fn stylesheet_text(
        &mut self,
        text: &str,
        base: &Url,
        budget: &mut Budget,
        stack: &mut HashSet<String>,
    ) -> String {
        if text.len() > MAX_BYTES.saturating_sub(budget.bytes) {
            self.log("stylesheet budget exceeded");
            return String::new();
        }
        budget.bytes += text.len();
        let sheet = syntax::parse_stylesheet(text);
        let mut out = Vec::new();
        let mut imports_allowed = true;
        for mut rule in sheet.rules {
            match &mut rule {
                Rule::At(at) if at.name.eq_ignore_ascii_case("import") => {
                    if !imports_allowed {
                        continue;
                    }
                    let prelude = syntax::trim(&at.prelude);
                    let Some(reference) = sol_css::parser::import_url(prelude.first()) else {
                        continue;
                    };
                    let Ok(url) = base.join(&reference) else {
                        continue;
                    };
                    let key = url.serialize();
                    if budget.imports >= 256 || stack.len() >= 16 || !stack.insert(key.clone()) {
                        self.log(&format!("stylesheet import limit or cycle: {url}"));
                        continue;
                    }
                    budget.imports += 1;
                    let result = self.client.borrow_mut().get(&url);
                    let mut imported = match result {
                        Ok(response) if response.status < 400 => {
                            let css = self.stylesheet_text(
                                &response.text(),
                                &response.url,
                                budget,
                                stack,
                            );
                            syntax::parse_stylesheet(&css).rules
                        }
                        Ok(response) => {
                            self.log(&format!("stylesheet {url}: HTTP {}", response.status));
                            Vec::new()
                        }
                        Err(error) => {
                            self.log(&format!("stylesheet {url}: {error}"));
                            Vec::new()
                        }
                    };
                    stack.remove(&key);
                    let mut remaining = syntax::trim(&prelude[1..]);
                    if let Some(value) = remaining.first() {
                        let layer = match value {
                            C::Token(token) if matches!(&token.kind,T::Ident(name) if name.eq_ignore_ascii_case("layer")) => {
                                Some(Vec::new())
                            }
                            C::Function { name, value } if name.eq_ignore_ascii_case("layer") => {
                                Some(value.clone())
                            }
                            _ => None,
                        };
                        if let Some(prelude) = layer {
                            imported = vec![Rule::At(syntax::AtRule {
                                name: "layer".into(),
                                prelude,
                                block: Some(rule_components(&imported)),
                            })];
                            remaining = syntax::trim(&remaining[1..]);
                        }
                    }
                    if let Some(C::Function { name, value }) = remaining.first() {
                        if name.eq_ignore_ascii_case("supports") {
                            let prelude = if value.iter().any(|v| v.kind() == Some(&T::Colon)) {
                                vec![C::Block {
                                    opening: '(',
                                    value: value.clone(),
                                }]
                            } else {
                                value.clone()
                            };
                            imported = vec![Rule::At(syntax::AtRule {
                                name: "supports".into(),
                                prelude,
                                block: Some(rule_components(&imported)),
                            })];
                            remaining = syntax::trim(&remaining[1..]);
                        }
                    }
                    if !remaining.is_empty() {
                        imported = vec![Rule::At(syntax::AtRule {
                            name: "media".into(),
                            prelude: remaining.to_vec(),
                            block: Some(rule_components(&imported)),
                        })];
                    }
                    out.extend(imported);
                    continue;
                }
                Rule::At(at) => {
                    if !(at.name.eq_ignore_ascii_case("charset")
                        || at.name.eq_ignore_ascii_case("layer") && at.block.is_none())
                    {
                        imports_allowed = false;
                    }
                    resolve(&mut at.prelude, base);
                    if let Some(block) = &mut at.block {
                        resolve(block, base);
                    }
                }
                Rule::Qualified(rule) => {
                    imports_allowed = false;
                    resolve(&mut rule.block, base);
                }
            }
            out.push(rule);
        }
        let out = out.iter().map(Rule::to_css).collect::<Vec<_>>().join("\n");
        if out.len() > MAX_BYTES {
            self.log("expanded stylesheet budget exceeded");
            String::new()
        } else {
            out
        }
    }
    pub(super) fn load_css_fonts(&mut self) {
        fn collect(rules: &[sol_css::Rule], out: &mut Vec<Vec<sol_css::Decl>>) {
            for rule in rules {
                match rule {
                    sol_css::Rule::FontFace(declarations) => out.push(declarations.clone()),
                    sol_css::Rule::Media(rule)
                        if sol_css::conditions::media_matches(&rule.queries) =>
                    {
                        collect(&rule.rules, out)
                    }
                    sol_css::Rule::Supports(rule) if rule.condition.supports() => {
                        collect(&rule.rules, out)
                    }
                    sol_css::Rule::Layer(rule) => collect(&rule.rules, out),
                    _ => {}
                }
            }
        }
        let mut faces = Vec::new();
        for sheet in &self.sheets {
            collect(&sheet.rules, &mut faces);
        }
        let mut bytes = 0usize;
        for face in faces {
            let property = |name: &str| {
                face.iter()
                    .rev()
                    .find(|d| d.name == name)
                    .map(|d| d.value.as_slice())
                    .unwrap_or_default()
            };
            let family = property("font-family")
                .iter()
                .filter_map(|v| match v {
                    sol_css::Value::Ident(s) | sol_css::Value::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            if family.is_empty() {
                continue;
            }
            let bold = property("font-weight").iter().any(|v| {
                matches!(v,sol_css::Value::Number(n) if *n>=600.0)
                    || matches!(v,sol_css::Value::Ident(s) if s=="bold")
            });
            let italic = property("font-style")
                .iter()
                .any(|v| matches!(v,sol_css::Value::Ident(s) if s=="italic"||s=="oblique"));
            for source in property("src") {
                let sol_css::Value::Url(source) = source else {
                    continue;
                };
                let Some(url) = self.resolve(source) else {
                    continue;
                };
                let response = self.client.borrow_mut().get(&url);
                match response {
                    Ok(response)
                        if response.status < 400
                            && response.body.len() <= 32 * 1024 * 1024
                            && response.body.len()
                                <= (64 * 1024 * 1024usize).saturating_sub(bytes) =>
                    {
                        let size = response.body.len();
                        match self.fonts.register(&family, response.body, bold, italic) {
                            Ok(()) => {
                                bytes += size;
                                break;
                            }
                            Err(error) => self.log(&format!("font {url}: {error}")),
                        }
                    }
                    Ok(response) => self.log(&format!(
                        "font {url}: HTTP {} or font budget exceeded",
                        response.status
                    )),
                    Err(error) => self.log(&format!("font {url}: {error}")),
                }
            }
        }
    }
}
fn rule_components(rules: &[Rule]) -> Vec<C> {
    syntax::parse_component_values(
        &rules
            .iter()
            .map(Rule::to_css)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .values
}
