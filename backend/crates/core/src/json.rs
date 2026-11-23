//! JSON spellings of values in user-facing messages, matching what `JSON.stringify` printed
//! in the TypeScript backend.

use serde_json::Value;

/// A string as a JSON string literal, with quotes and escapes.
pub(crate) fn quote(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_print_as_javascript_prints_them() {
        assert_eq!(quote("my robot"), "\"my robot\"");
        assert_eq!(quote("a\"b\n"), "\"a\\\"b\\n\"");
        assert_eq!(quote("robůtek"), "\"robůtek\"");
    }
}
