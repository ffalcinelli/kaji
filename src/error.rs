//! Error types shared across kaji.
//!
//! Errors are `anyhow::Error` chains. Each layer adds only the context it owns
//! (realm → stage or resource type → file → operation), the HTTP layer ends the chain with an
//! [`HttpError`], causes starting with `Hint:` suggest a fix, and independent failures are
//! collected in a [`MultiError`]. Code must inspect errors through the typed helpers below,
//! never by matching message text.

use reqwest::StatusCode;
use serde::Deserialize;
use std::fmt;

/// Maximum length of a raw (non-JSON) response body quoted in an error.
const MAX_DETAIL_LEN: usize = 300;

/// A non-success HTTP response from Keycloak.
#[derive(Debug, Clone)]
pub struct HttpError {
    /// HTTP method of the request.
    pub method: String,
    /// Request URL, with credentials redacted.
    pub url: String,
    /// Response status.
    pub status: StatusCode,
    /// Error message extracted from the response body (may be empty).
    pub detail: String,
}

impl HttpError {
    /// Builds the error from a response body, extracting Keycloak's error message.
    pub fn new(
        method: impl Into<String>,
        url: impl Into<String>,
        status: StatusCode,
        body: &str,
    ) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            status,
            detail: extract_detail(body),
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} failed: {}", self.method, self.url, self.status)?;
        if !self.detail.is_empty() {
            write!(f, " - {}", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {}

/// Extracts a readable message from an error response body: Keycloak's JSON error fields, or
/// the raw body (whitespace collapsed, truncated). HTML error pages are dropped.
fn extract_detail(body: &str) -> String {
    #[derive(Deserialize)]
    struct KeycloakErrorBody {
        error: Option<String>,
        error_description: Option<String>,
        #[serde(rename = "errorMessage")]
        error_message: Option<String>,
    }

    if let Ok(err_body) = serde_json::from_str::<KeycloakErrorBody>(body) {
        let detail = err_body
            .error_description
            .or(err_body.error_message)
            .or(err_body.error)
            .unwrap_or_default();
        if !detail.is_empty() {
            return detail;
        }
    }
    let trimmed = body.trim();
    if trimmed.starts_with('<') {
        return String::new();
    }
    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_DETAIL_LEN {
        let truncated: String = collapsed.chars().take(MAX_DETAIL_LEN).collect();
        format!("{}…", truncated)
    } else {
        collapsed
    }
}

/// Several independent failures (e.g. one per resource file), each with its own context chain.
#[derive(Debug)]
pub struct MultiError {
    errors: Vec<anyhow::Error>,
    /// What the errors are, after their count (e.g. `operations failed`).
    noun: &'static str,
}

impl MultiError {
    /// Collects `errors`, sorted by message so the output does not depend on task timing.
    pub fn new(errors: Vec<anyhow::Error>) -> Self {
        Self::with_noun(errors, "operations failed")
    }

    /// Like [`MultiError::new`], summarized as `{count} {noun}` (e.g. `3 problems found`).
    pub fn with_noun(mut errors: Vec<anyhow::Error>, noun: &'static str) -> Self {
        errors.sort_by_cached_key(|e| format!("{:#}", e));
        Self { errors, noun }
    }

    /// The collected failures.
    pub fn errors(&self) -> &[anyhow::Error] {
        &self.errors
    }

    /// The collected failures, by value.
    pub fn into_errors(self) -> Vec<anyhow::Error> {
        self.errors
    }

    /// The summary line, e.g. `2 operations failed:`.
    pub fn summary(&self) -> String {
        format!("{} {}:", self.errors.len(), self.noun)
    }

    /// Returns `Ok(())` for no errors, the error itself for one, and a `MultiError` otherwise.
    pub fn into_result(errors: Vec<anyhow::Error>) -> anyhow::Result<()> {
        Self::into_result_with_noun(errors, "operations failed")
    }

    /// [`MultiError::into_result`] with the summary noun of [`MultiError::with_noun`].
    pub fn into_result_with_noun(
        mut errors: Vec<anyhow::Error>,
        noun: &'static str,
    ) -> anyhow::Result<()> {
        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.remove(0)),
            _ => Err(Self::with_noun(errors, noun).into()),
        }
    }

    /// Flattens nested `MultiError`s so the failures are listed at one level.
    pub fn flatten(errors: Vec<anyhow::Error>) -> Vec<anyhow::Error> {
        let mut flat = Vec::new();
        for error in errors {
            match error.downcast::<MultiError>() {
                Ok(multi) => flat.extend(Self::flatten(multi.into_errors())),
                Err(error) => flat.push(error),
            }
        }
        flat
    }
}

impl fmt::Display for MultiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())?;
        for (i, err) in self.errors.iter().enumerate() {
            write!(f, "\n  {}. {:#}", i + 1, err)?;
        }
        Ok(())
    }
}

impl std::error::Error for MultiError {}

/// The status of the first [`HttpError`] in the chain, if any.
pub fn http_status(err: &anyhow::Error) -> Option<StatusCode> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<HttpError>())
        .map(|e| e.status)
}

/// Returns true if the error chain contains an HTTP 404 response from Keycloak.
pub fn is_not_found(err: &anyhow::Error) -> bool {
    http_status(err) == Some(StatusCode::NOT_FOUND)
}

/// Returns true if the error chain contains an HTTP 409 response from Keycloak.
pub fn is_conflict(err: &anyhow::Error) -> bool {
    http_status(err) == Some(StatusCode::CONFLICT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn test_http_error_display() {
        let err = HttpError::new(
            "GET",
            "https://kc/admin/realms/r/clients",
            StatusCode::NOT_FOUND,
            r#"{"error":"Could not find client"}"#,
        );
        assert_eq!(
            err.to_string(),
            "GET https://kc/admin/realms/r/clients failed: 404 Not Found - Could not find client"
        );

        let err = HttpError::new("POST", "https://kc/x", StatusCode::BAD_GATEWAY, "  ");
        assert_eq!(err.to_string(), "POST https://kc/x failed: 502 Bad Gateway");

        let html = "<html><body>Bad gateway</body></html>";
        let err = HttpError::new("GET", "https://kc/x", StatusCode::BAD_GATEWAY, html);
        assert!(err.detail.is_empty());
    }

    #[test]
    fn test_extract_detail() {
        assert_eq!(
            extract_detail(r#"{"error":"invalid_grant","error_description":"Bad creds"}"#),
            "Bad creds"
        );
        assert_eq!(extract_detail(r#"{"errorMessage":"Exists"}"#), "Exists");
        assert_eq!(extract_detail(r#"{"other":1}"#), r#"{"other":1}"#);
        assert_eq!(extract_detail("line one\n  line two"), "line one line two");
        let long = "x".repeat(MAX_DETAIL_LEN + 10);
        assert_eq!(
            extract_detail(&long).chars().count(),
            MAX_DETAIL_LEN + 1 // ellipsis
        );
    }

    #[test]
    fn test_status_helpers_see_through_context() {
        let err = anyhow::Error::new(HttpError::new("GET", "u", StatusCode::NOT_FOUND, ""))
            .context("Failed to fetch clients")
            .context("Failed to plan realm 'r'");
        assert!(is_not_found(&err));
        assert!(!is_conflict(&err));

        let err: anyhow::Error = HttpError::new("POST", "u", StatusCode::CONFLICT, "").into();
        assert!(is_conflict(&err));

        let err = anyhow::anyhow!("404 Not Found (plain text is not an HTTP error)");
        assert!(!is_not_found(&err));
        assert_eq!(http_status(&err), None);
    }

    #[test]
    fn test_multi_error() {
        assert!(MultiError::into_result(vec![]).is_ok());

        let single = MultiError::into_result(vec![anyhow::anyhow!("only")]).unwrap_err();
        assert_eq!(single.to_string(), "only");

        let err = MultiError::into_result(vec![
            Err::<(), _>(anyhow::anyhow!("root b"))
                .context("second")
                .unwrap_err(),
            anyhow::anyhow!("first"),
        ])
        .unwrap_err();
        let multi = err.downcast_ref::<MultiError>().unwrap();
        assert_eq!(multi.errors().len(), 2);
        assert_eq!(
            err.to_string(),
            "2 operations failed:\n  1. first\n  2. second: root b"
        );
    }
}
