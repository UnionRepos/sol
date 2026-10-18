use crate::syntax::{self, ComponentValue as C, TokenKind as T};

#[derive(Clone, Copy, PartialEq)]
enum Dimension {
    Number,
    Length,
}
#[derive(Clone, Copy)]
struct Quantity {
    value: f64,
    dimension: Dimension,
}
struct Math<'a> {
    values: &'a [C],
    pos: usize,
    font: f64,
    percent: f64,
}

pub(crate) fn length(name: &str, values: &[C], font: f64, percent: f64) -> Option<f64> {
    let result = function(name, values, font, percent)?;
    (result.dimension == Dimension::Length || result.value == 0.0).then_some(result.value)
}
fn function(name: &str, values: &[C], font: f64, percent: f64) -> Option<Quantity> {
    let evaluate = |values: &[C]| {
        let mut parser = Math {
            values: syntax::trim(values),
            pos: 0,
            font,
            percent,
        };
        let result = parser.sum()?;
        parser.ws();
        (parser.pos == parser.values.len() && result.value.is_finite()).then_some(result)
    };
    match name {
        "calc" => evaluate(values),
        "min" | "max" | "clamp" => {
            let groups = syntax::split(values, &T::Comma);
            if name == "clamp" && groups.len() != 3 {
                return None;
            }
            let quantities: Vec<_> = groups.into_iter().map(evaluate).collect::<Option<_>>()?;
            let dimension = quantities.first()?.dimension;
            if quantities.iter().any(|q| q.dimension != dimension) {
                return None;
            }
            let value = match name {
                "min" => quantities
                    .iter()
                    .map(|q| q.value)
                    .fold(f64::INFINITY, f64::min),
                "max" => quantities
                    .iter()
                    .map(|q| q.value)
                    .fold(f64::NEG_INFINITY, f64::max),
                _ => quantities[1]
                    .value
                    .min(quantities[2].value)
                    .max(quantities[0].value),
            };
            Some(Quantity { value, dimension })
        }
        _ => None,
    }
}
impl Math<'_> {
    fn ws(&mut self) -> bool {
        let start = self.pos;
        while self.values.get(self.pos).is_some_and(C::is_whitespace) {
            self.pos += 1;
        }
        start != self.pos
    }
    fn sum(&mut self) -> Option<Quantity> {
        let mut result = self.product()?;
        loop {
            let spaced = self.ws();
            let op = self.values.get(self.pos).and_then(C::kind);
            let sign = match op {
                Some(T::Delim('+')) => 1.0,
                Some(T::Delim('-')) => -1.0,
                _ => break,
            };
            // Whitespace is part of the sum grammar; erasing it before parsing
            // would accept expressions whose sign belongs to a numeric token.
            if !spaced {
                return None;
            }
            self.pos += 1;
            if !self.ws() {
                return None;
            }
            let rhs = self.product()?;
            if rhs.dimension != result.dimension {
                return None;
            }
            result.value += sign * rhs.value;
        }
        Some(result)
    }
    fn product(&mut self) -> Option<Quantity> {
        let mut result = self.atom()?;
        loop {
            let save = self.pos;
            self.ws();
            let op = self.values.get(self.pos).and_then(C::kind);
            let divide = match op {
                Some(T::Delim('*')) => false,
                Some(T::Delim('/')) => true,
                _ => {
                    self.pos = save;
                    break;
                }
            };
            self.pos += 1;
            let rhs = self.atom()?;
            if divide {
                if rhs.dimension != Dimension::Number || rhs.value == 0.0 {
                    return None;
                }
                result.value /= rhs.value;
            } else {
                if result.dimension == Dimension::Length && rhs.dimension == Dimension::Length {
                    return None;
                }
                if rhs.dimension == Dimension::Length {
                    result.dimension = Dimension::Length;
                }
                result.value *= rhs.value;
            }
        }
        Some(result)
    }
    fn atom(&mut self) -> Option<Quantity> {
        self.ws();
        let value = self.values.get(self.pos)?;
        self.pos += 1;
        match value {
            C::Token(token) => match &token.kind {
                T::Number(number) => Some(Quantity {
                    value: number.value,
                    dimension: Dimension::Number,
                }),
                T::Percentage(number) => Some(Quantity {
                    value: number.value * self.percent / 100.0,
                    dimension: Dimension::Length,
                }),
                T::Dimension(..) => {
                    let values = crate::values::from_components(std::slice::from_ref(value));
                    Some(Quantity {
                        value: crate::style::value_px(values.first()?, self.font, self.percent)?,
                        dimension: Dimension::Length,
                    })
                }
                _ => None,
            },
            C::Function { name, value } => {
                function(&name.to_ascii_lowercase(), value, self.font, self.percent)
            }
            C::Block {
                opening: '(',
                value,
            } => function("calc", value, self.font, self.percent),
            _ => None,
        }
    }
}
