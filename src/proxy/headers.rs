use crate::config::InjectionMode;
use hyper::{
    HeaderMap,
    header::{CONNECTION, HeaderName, HeaderValue, PROXY_AUTHORIZATION, UPGRADE},
};

pub(super) fn strip(headers: &mut HeaderMap) {
    let dynamic: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| name.trim().parse().ok())
        .collect();
    for name in dynamic {
        headers.remove(name);
    }
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

pub(super) fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(UPGRADE)
        || headers.get_all(CONNECTION).iter().any(|value| {
            value.to_str().is_ok_and(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            })
        })
}

pub(super) fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let mut values = headers.get_all(PROXY_AUTHORIZATION).iter();
    let Some(value) = values.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    let Some((scheme, supplied)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return false;
    }
    // Avoid early mismatch exits for equal-length tokens. Length is not secret.
    supplied.len() == token.len()
        && supplied
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

pub(super) fn inject(
    headers: &mut HeaderMap,
    mode: InjectionMode,
    auth: &HeaderValue,
    key: &HeaderValue,
) {
    let has_key = ["authorization", "x-api-key"]
        .iter()
        .any(|name| headers.get(*name).is_some_and(|value| !value.is_empty()));
    if mode == InjectionMode::Off || (mode == InjectionMode::Missing && has_key) {
        return;
    }
    let anthropic = headers
        .get("x-api-key")
        .is_some_and(|value| !value.is_empty())
        || headers
            .get("anthropic-version")
            .is_some_and(|value| !value.is_empty());
    headers.insert("authorization", auth.clone());
    if anthropic {
        headers.insert("x-api-key", key.clone());
    }
}
