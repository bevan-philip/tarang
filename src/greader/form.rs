use axum::extract::{FromRequest, Request};
use axum::http::{Method, header};
use std::collections::HashMap;

use super::GReaderError;

/// Merges GET query-string params and POST form-body params into one map,
/// since the Google Reader protocol reads params from whichever location
/// matches the HTTP method and some calls repeat the same key (e.g. `a=`
/// appears once per tag in `edit-tag`).
#[derive(Debug, Default, Clone)]
pub struct MergedParams(HashMap<String, Vec<String>>);

impl MergedParams {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.first()).map(|s| s.as_str())
    }

    pub fn get_all(&self, key: &str) -> &[String] {
        self.0.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn merge(&mut self, query: &str) {
        for (k, v) in form_urlencoded::parse(query.as_bytes()) {
            self.0
                .entry(k.into_owned())
                .or_default()
                .push(v.into_owned());
        }
    }

    #[cfg(test)]
    pub(crate) fn from_query(query: &str) -> Self {
        let mut params = Self::default();
        params.merge(query);
        params
    }
}

impl<S> FromRequest<S> for MergedParams
where
    S: Send + Sync,
{
    type Rejection = GReaderError;

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let mut params = MergedParams::default();

        if let Some(query) = req.uri().query() {
            params.merge(query);
        }

        if req.method() == Method::POST {
            let is_form = req
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.starts_with("application/x-www-form-urlencoded"))
                .unwrap_or(false);

            if is_form {
                let body = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .map_err(|e| GReaderError::BadRequest(format!("failed to read body: {e}")))?;
                params.merge(std::str::from_utf8(&body).unwrap_or(""));
            }
        }

        Ok(params)
    }
}
