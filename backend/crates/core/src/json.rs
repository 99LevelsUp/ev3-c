//! JSON spellings of values in user-facing messages, matching what `JSON.stringify` printed
//! in the TypeScript backend.

use serde_json::Value;

/// A string as a JSON string literal, with quotes and escapes.
pub(crate) fn quote(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

/// Any JSON value as `JSON.stringify` prints it. JavaScript has one number type, so a
/// whole number read as a float (`5.0`, `1e3`) prints without a fraction.
pub(crate) fn stringify(value: &Value) -> String {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() && float.fract() == 0.0 && float.abs() < 1e21 => {
                // JSON.stringify(-0) prints "0".
                format!("{:.0}", float + 0.0)
            }
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_numbers_print_as_javascript_prints_them() {
        assert_eq!(quote("my robot"), "\"my robot\"");
        assert_eq!(quote("a\"b\n"), "\"a\\\"b\\n\"");
        assert_eq!(quote("robůtek"), "\"robůtek\"");
        let parse = |text: &str| serde_json::from_str::<Value>(text).unwrap();
        assert_eq!(stringify(&parse("5.0")), "5");
        assert_eq!(stringify(&parse("1e3")), "1000");
        assert_eq!(stringify(&parse("-0.0")), "0");
        assert_eq!(stringify(&parse("2.5")), "2.5");
        assert_eq!(stringify(&parse("-1")), "-1");
        assert_eq!(stringify(&parse("\"5\"")), "\"5\"");
        assert_eq!(stringify(&parse("null")), "null");
    }
}
