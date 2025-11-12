use crate::error::{AppError, Result};
use url::Url;

pub fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn parse_collection_id(input: &str) -> Result<u32> {
    let trimmed = input.trim();

    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return trimmed.parse::<u32>()
            .map_err(|_| AppError::invalid_url_dynamic(
                format!("Invalid collection ID: {}", trimmed).into_boxed_str()
            ));
    }

    if trimmed.is_empty() {
        return Err(AppError::invalid_url(
            "Collection ID or URL cannot be empty"
        ));
    }

    let url = Url::parse(trimmed)
        .map_err(|_| AppError::invalid_url_dynamic(
            format!("Invalid URL or collection ID: {}", trimmed).into_boxed_str()
        ))?;

    if url.host_str() != Some("osucollector.com") {
        return Err(AppError::invalid_url(
            "URL must be from osucollector.com"
        ));
    }

    if url.scheme() != "https" {
        return Err(AppError::invalid_url(
            "URL must use HTTPS protocol"
        ));
    }

    let path_segments: Vec<&str> = url.path_segments()
        .ok_or(AppError::invalid_url("Invalid URL path"))?
        .collect();

    if path_segments.len() < 2 || path_segments[0] != "collections" {
        return Err(AppError::invalid_url(
            "URL must be in format: https://osucollector.com/collections/{id}"
        ));
    }

    let id = path_segments[1];

    id.parse::<u32>()
        .map_err(|_| AppError::invalid_url_dynamic(
            format!("Collection ID must be numeric, got: {}", id).into_boxed_str()
        ))
}
