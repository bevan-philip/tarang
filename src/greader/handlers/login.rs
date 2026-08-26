use axum::Json;
use axum::http::header;
use axum::response::IntoResponse;

use crate::greader::form::MergedParams;
use crate::greader::responses::UserInfoResponse;

/// Auth is a formality here, not a security boundary (tarang is
/// single-tenant, trusted-network only) — every client gets the same fixed
/// token, and no endpoint actually checks it.
const GREADER_TOKEN: &str = "tarang-greader-token";

pub async fn client_login(_params: MergedParams) -> impl IntoResponse {
    let body = format!("SID={GREADER_TOKEN}\nLSID={GREADER_TOKEN}\nAuth={GREADER_TOKEN}\n");
    ([(header::CONTENT_TYPE, "text/plain")], body)
}

pub async fn token() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/plain")], GREADER_TOKEN)
}

pub async fn user_info() -> Json<UserInfoResponse> {
    Json(UserInfoResponse {
        user_id: "1".to_string(),
        user_name: "tarang".to_string(),
        user_profile_id: "1".to_string(),
        user_email: "tarang@localhost".to_string(),
    })
}
