pub(crate) fn expand_environment_strings(text: &str) -> String {
    expand_environment_strings_with(text, |name| std::env::var(name).ok())
}

fn expand_environment_strings_with<F>(text: &str, mut lookup: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    if !text.contains('%') {
        return text.to_string();
    }

    let mut output = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('%') else {
            output.push('%');
            output.push_str(after_start);
            return output;
        };

        let name = &after_start[..end];
        if name.is_empty() {
            // Keep the first percent literal and reconsider the second one as
            // the start of a possible variable, matching Windows expansion.
            output.push('%');
            rest = after_start;
            continue;
        } else if let Some(value) = lookup(name) {
            output.push_str(&value);
        } else {
            output.push('%');
            output.push_str(name);
            output.push('%');
        }

        rest = &after_start[end + 1..];
    }

    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_environment(name: &str) -> Option<String> {
        if name.eq_ignore_ascii_case("USERPROFILE") {
            Some(r"C:\Users\Test".to_string())
        } else if name.eq_ignore_ascii_case("LOCALAPPDATA") {
            Some(r"C:\Users\Test\AppData\Local".to_string())
        } else {
            None
        }
    }

    #[test]
    fn expands_environment_variables_case_insensitively() {
        assert_eq!(
            expand_environment_strings_with(
                r"%userprofile%\AppData\Local\%LOCALAPPDATA%",
                test_environment,
            ),
            r"C:\Users\Test\AppData\Local\C:\Users\Test\AppData\Local"
        );
    }

    #[test]
    fn production_lookup_is_case_insensitive_on_windows() {
        let Some(expected) = std::env::var("USERPROFILE").ok() else {
            return;
        };

        assert_eq!(expand_environment_strings("%userprofile%"), expected);
    }

    #[test]
    fn expands_multiple_adjacent_variables() {
        assert_eq!(
            expand_environment_strings_with(r"%USERPROFILE%%LOCALAPPDATA%", test_environment,),
            r"C:\Users\TestC:\Users\Test\AppData\Local"
        );
    }

    #[test]
    fn preserves_unknown_empty_and_unclosed_variables() {
        assert_eq!(
            expand_environment_strings_with(r"%UNKNOWN%\%%\%USERPROFILE", test_environment,),
            r"%UNKNOWN%\%%\%USERPROFILE"
        );
    }

    #[test]
    fn reconsiders_second_percent_as_a_variable_start() {
        assert_eq!(
            expand_environment_strings_with(r"%%USERPROFILE%", test_environment),
            r"%C:\Users\Test"
        );
        assert_eq!(
            expand_environment_strings_with(r"%USERPROFILE%%", test_environment),
            r"C:\Users\Test%"
        );
        assert_eq!(
            expand_environment_strings_with("%%", test_environment),
            "%%"
        );
    }

    #[test]
    fn preserves_text_without_variables() {
        assert_eq!(
            expand_environment_strings_with(r"C:\Users\Test", test_environment),
            r"C:\Users\Test"
        );
    }
}
