const MAX_TEXT_FIELD_LEN: usize = 240;

pub(super) fn sanitize_log_message(input: &str) -> String {
    let redacted_quotes = redact_quoted_segments(input);
    let mut output = Vec::new();

    for token in redacted_quotes.split_whitespace() {
        let (prefix, core, suffix) = split_wrapping_punctuation(token);
        if core.is_empty() {
            output.push(token.to_string());
            continue;
        }

        if is_redaction_placeholder(core) {
            output.push(token.to_string());
            continue;
        }

        if let Some((key, _)) = core.split_once('=') {
            if is_sensitive_key(key) {
                output.push(format!("{prefix}{key}=<redacted>{suffix}"));
                break;
            }
        }

        if !core.contains("::") {
            if let Some((key, _)) = core.split_once(':') {
                if is_sensitive_key(key) {
                    output.push(format!("{prefix}{key}:<redacted>{suffix}"));
                    break;
                }
            }
        }

        if let Some(key) = core.strip_suffix(':') {
            if is_sensitive_key(key) {
                output.push(format!("{prefix}{key}:<redacted>{suffix}"));
                break;
            }
        }

        if is_sensitive_context_word(core) {
            output.clear();
            output.push(if core.eq_ignore_ascii_case("drive") {
                "<drive>".to_string()
            } else {
                "<redacted>".to_string()
            });
            break;
        }

        if looks_like_path(core) {
            if looks_like_absolute_path(core) {
                output.push(format!("{prefix}<path>{suffix}"));
            } else {
                // A relative path may have earlier components containing spaces.
                // Discard the whole message rather than risk retaining one.
                output.clear();
                output.push("<path>".to_string());
            }
            break;
        }

        if is_drive_designator(core) {
            output.push(format!("{prefix}<drive>{suffix}"));
            break;
        }

        if looks_like_sensitive_identifier(core) {
            output.push(format!("{prefix}<redacted>{suffix}"));
        } else if looks_like_filename(core) {
            // Without quotes or a field key, earlier words could be parts of
            // a filename containing spaces. Discard this whole free-text
            // message instead of retaining a partial name.
            output.clear();
            output.push("<file>".to_string());
            break;
        } else {
            output.push(token.to_string());
        }
    }

    truncate_text(&output.join(" "))
}

fn redact_quoted_segments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        let quote = chars[index];
        if quote != '\'' && quote != '"' {
            out.push(quote);
            index += 1;
            continue;
        }

        out.push_str("<redacted>");
        index += 1;
        let mut escaped = false;
        while index < chars.len() {
            let ch = chars[index];
            index += 1;
            if quote == '"' && escaped {
                escaped = false;
                continue;
            }
            if quote == '"' && ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                break;
            }
        }
    }

    out
}

fn split_wrapping_punctuation(token: &str) -> (&str, &str, &str) {
    let trim_chars: &[char] = &['(', ')', '[', ']', '{', '}', ',', ';'];
    let core = token.trim_matches(trim_chars);
    let start = token.find(core).unwrap_or(0);
    let end = start.saturating_add(core.len());
    (&token[..start], core, &token[end..])
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key
        .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if matches!(
        key.as_str(),
        "files"
            | "file_count"
            | "file_ops"
            | "file_operations"
            | "drive_count"
            | "drive_type"
            | "volume_count"
            | "volume_type"
            | "device_type"
    ) {
        return false;
    }

    let parts: Vec<_> = key.split(['_', '-', '.']).collect();
    parts.iter().any(|part| {
        matches!(
            *part,
            "path"
                | "filename"
                | "file_name"
                | "file"
                | "folder"
                | "directory"
                | "query"
                | "search"
                | "name"
                | "user"
                | "username"
                | "account"
                | "email"
                | "token"
                | "credential"
                | "password"
                | "drive"
                | "volume"
                | "label"
                | "serial"
                | "server"
                | "host"
                | "hostname"
                | "computer"
                | "machine"
                | "device"
                | "uuid"
                | "guid"
                | "sid"
                | "mac"
                | "hash"
                | "address"
                | "ip"
        )
    })
}

fn is_sensitive_context_word(word: &str) -> bool {
    matches!(
        word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
            .to_ascii_lowercase()
            .as_str(),
        "drive"
            | "volume"
            | "label"
            | "path"
            | "file"
            | "filename"
            | "folder"
            | "directory"
            | "name"
            | "query"
            | "search"
            | "username"
            | "account"
            | "email"
            | "hostname"
            | "computer"
            | "machine"
            | "serial"
            | "server"
            | "host"
            | "device"
            | "token"
            | "credential"
            | "password"
    )
}

fn is_redaction_placeholder(token: &str) -> bool {
    ["<redacted>", "<path>", "<file>", "<drive>"]
        .iter()
        .any(|placeholder| {
            token.strip_prefix(placeholder).is_some_and(|suffix| {
                suffix
                    .chars()
                    .all(|ch| matches!(ch, ':' | '.' | '!' | '?' | ',' | ';' | ')' | ']' | '}'))
            })
        })
}

fn looks_like_path(fragment: &str) -> bool {
    fragment.contains('/') || fragment.contains('\\')
}

fn looks_like_absolute_path(fragment: &str) -> bool {
    fragment.starts_with("\\\\")
        || fragment.starts_with("//")
        || fragment.starts_with('/')
        || is_windows_drive_path(fragment)
}

fn is_windows_drive_path(fragment: &str) -> bool {
    let bytes = fragment.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn is_drive_designator(fragment: &str) -> bool {
    let bytes = fragment.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn looks_like_sensitive_identifier(fragment: &str) -> bool {
    fragment.contains('@')
        || looks_like_ipv4(fragment)
        || looks_like_ipv6(fragment)
        || looks_like_mac_address(fragment)
        || looks_like_sid(fragment)
        || looks_like_guid(fragment)
}

fn looks_like_ipv6(fragment: &str) -> bool {
    fragment.matches(':').count() >= 2
        && !fragment.contains(":::")
        && fragment
            .chars()
            .all(|ch| ch == ':' || ch == '.' || ch.is_ascii_hexdigit())
}

fn looks_like_ipv4(fragment: &str) -> bool {
    let octets: Vec<_> = fragment.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|octet| {
            !octet.is_empty()
                && octet.len() <= 3
                && octet.bytes().all(|byte| byte.is_ascii_digit())
                && octet.parse::<u8>().is_ok()
        })
}

fn looks_like_mac_address(fragment: &str) -> bool {
    let parts: Vec<_> = fragment.split([':', '-']).collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn looks_like_sid(fragment: &str) -> bool {
    let parts: Vec<_> = fragment.split('-').collect();
    parts.len() >= 4
        && parts[0].eq_ignore_ascii_case("S")
        && parts[1..]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn looks_like_guid(fragment: &str) -> bool {
    let parts: Vec<_> = fragment.split('-').collect();
    matches!(
        parts
            .iter()
            .map(|part| part.len())
            .collect::<Vec<_>>()
            .as_slice(),
        [8, 4, 4, 4, 12]
    ) && parts
        .iter()
        .all(|part| part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn looks_like_filename(fragment: &str) -> bool {
    let trimmed = fragment.trim_end_matches(':').trim_matches('.');
    if trimmed.contains("::") || !trimmed.contains('.') {
        return false;
    }

    let Some((base, extension)) = trimmed.rsplit_once('.') else {
        return false;
    };

    !base.is_empty()
        && base.chars().any(|ch| !ch.is_whitespace())
        && !extension.is_empty()
        && extension.len() <= 16
        && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        && extension.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn truncate_text(input: &str) -> String {
    input.chars().take(MAX_TEXT_FIELD_LEN).collect()
}

#[cfg(test)]
mod tests;
