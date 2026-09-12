use crate::Issue;

/// Intentionally a literal grammar, not a MATLAB expression evaluator.
pub(crate) fn vector(text: &str) -> Result<Vec<f64>, Issue> {
    let trimmed = text.trim();
    let (body, bracketed) =
        if let Some(body) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            (body, true)
        } else {
            (trimmed, false)
        };
    let multi_row = body.contains(';');
    let mut values = Vec::new();
    for row in body.split(';') {
        let mut row_items = 0;
        for group in row.split(',') {
            if group.trim().is_empty() {
                return Err(shape());
            }
            for item in group.split_ascii_whitespace() {
                row_items += 1;
                if multi_row && row_items > 1 {
                    return Err(shape());
                }
                if !item
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
                {
                    return Err(expression());
                }
                let value: f64 = item.parse().map_err(|_| expression())?;
                if !value.is_finite() {
                    return Err(expression());
                }
                values.push(value);
                if values.len() > 262_144 {
                    return Err(Issue::new(
                        "literal_limit",
                        "literal exceeds component limit",
                    ));
                }
            }
        }
    }
    if !bracketed && values.len() != 1 {
        return Err(expression());
    }
    Ok(values)
}
fn shape() -> Issue {
    Issue::new(
        "literal_shape",
        "only scalar and one-dimensional vector literals are supported",
    )
}
pub(crate) fn scalar(text: &str) -> Result<f64, Issue> {
    let values = vector(text)?;
    if values.len() != 1 {
        return Err(Issue::new("literal_shape", "expected a scalar literal"));
    }
    Ok(values[0])
}
fn expression() -> Issue {
    Issue::new(
        "parameter_expression",
        "parameter requires a finite numeric literal; expressions and workspace variables are not evaluated during import",
    )
}
