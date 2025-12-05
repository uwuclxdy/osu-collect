pub fn sanitize_filename(filename: &str) -> String {
    let mut sanitized = String::with_capacity(filename.len());

    for c in filename.chars() {
        sanitized.push(match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        });
    }

    let mut start = sanitized.len();
    let mut end = 0;

    for (idx, ch) in sanitized.char_indices() {
        if !ch.is_whitespace() {
            start = idx;
            break;
        }
    }

    for (idx, ch) in sanitized.char_indices().rev() {
        if !ch.is_whitespace() {
            end = idx + ch.len_utf8();
            break;
        }
    }

    if start >= end {
        sanitized.clear();
    } else {
        sanitized.drain(..start);
        sanitized.truncate(end - start);
    }

    sanitized
}
