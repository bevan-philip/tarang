pub fn client_builder() -> reqwest::ClientBuilder {
    let tag = option_env!("TARANG_RELEASE_VERSION").unwrap_or("");
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let product = if version.is_empty() {
        "tarang".to_string()
    } else {
        format!("tarang/{version}")
    };

    reqwest::Client::builder().user_agent(format!(
        "{product} (+https://github.com/bevan-philip/tarang)"
    ))
}
