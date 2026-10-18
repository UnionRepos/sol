use crate::Value;

pub(crate) fn expand(values: &[Value]) -> Vec<(String, Vec<Value>)> {
    let properties = [
        "image",
        "position",
        "size",
        "repeat",
        "attachment",
        "origin",
        "clip",
    ];
    let mut result: Vec<Vec<Value>> = vec![Vec::new(); properties.len()];
    let mut color = vec![Value::Color(0, 0, 0, 0)];
    for (index, layer) in values
        .split(|v| matches!(v,Value::Ident(s) if s==","))
        .enumerate()
    {
        let mut image = vec![Value::Ident("none".into())];
        let mut position = Vec::new();
        let mut size = Vec::new();
        let mut repeat = Vec::new();
        let mut boxes = Vec::new();
        let mut attachment = vec![Value::Ident("scroll".into())];
        let mut after_slash = false;
        for value in layer {
            match value {
                Value::Color(..) => color = vec![value.clone()],
                Value::Url(_) => image = vec![value.clone()],
                Value::Func(name, _) if name.ends_with("gradient") || name == "url" => {
                    image = vec![value.clone()]
                }
                Value::Ident(name) if name == "none" => image = vec![value.clone()],
                Value::Ident(name) if name == "/" => after_slash = true,
                Value::Ident(name)
                    if matches!(
                        name.as_str(),
                        "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round"
                    ) =>
                {
                    repeat.push(value.clone());
                    after_slash = false;
                }
                Value::Ident(name) if matches!(name.as_str(), "fixed" | "local" | "scroll") => {
                    attachment = vec![value.clone()];
                    after_slash = false;
                }
                Value::Ident(name) if name.ends_with("-box") => {
                    boxes.push(value.clone());
                    after_slash = false;
                }
                Value::Ident(name) if name == "currentcolor" => color = vec![value.clone()],
                _ if after_slash => size.push(value.clone()),
                _ => position.push(value.clone()),
            }
        }
        if position.is_empty() {
            position = vec![Value::Percentage(0.0), Value::Percentage(0.0)];
        }
        if size.is_empty() {
            size = vec![Value::Ident("auto".into())];
        }
        if repeat.is_empty() {
            repeat = vec![Value::Ident("repeat".into())];
        }
        let origin = vec![boxes
            .first()
            .cloned()
            .unwrap_or(Value::Ident("padding-box".into()))];
        let clip = vec![boxes
            .get(1)
            .or_else(|| boxes.first())
            .cloned()
            .unwrap_or(Value::Ident("border-box".into()))];
        for (target, values) in result
            .iter_mut()
            .zip([image, position, size, repeat, attachment, origin, clip])
        {
            if index > 0 {
                target.push(Value::Ident(",".into()));
            }
            target.extend(values);
        }
    }
    let mut out: Vec<_> = properties
        .into_iter()
        .zip(result)
        .map(|(name, value)| (format!("background-{name}"), value))
        .collect();
    out.push(("background-color".into(), color));
    out
}
