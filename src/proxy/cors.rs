//! Explicit browser policy; preflight never opens an upstream connection.
use super::{Body, reply};
use crate::config::CorsConfig;
use hyper::{
    HeaderMap, Method, Response, StatusCode,
    header::{HeaderName, HeaderValue},
};

pub(super) struct Cors {
    enabled: bool,
    origin: Option<HeaderValue>,
    credentials: bool,
    wildcard: bool,
}
const METHODS: &str = "GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS";
impl Cors {
    pub(super) fn new(config: &CorsConfig, headers: &HeaderMap) -> Self {
        let mut origins = headers.get_all("origin").iter();
        let origin = origins.next().filter(|_| origins.next().is_none());
        let origin = origin.filter(|value| {
            value.to_str().is_ok_and(|value| {
                config
                    .allowed_origins
                    .iter()
                    .any(|allowed| allowed == value || allowed == "*")
            })
        });
        Self {
            enabled: config.enabled,
            origin: if config.enabled {
                origin.cloned()
            } else {
                None
            },
            credentials: config.allow_credentials,
            wildcard: config.allowed_origins.iter().any(|o| o == "*"),
        }
    }
    pub(super) fn preflight(&self, method: &Method, headers: &HeaderMap) -> Option<Response<Body>> {
        if !self.enabled
            || method != Method::OPTIONS
            || !headers.contains_key("origin")
            || !headers.contains_key("access-control-request-method")
        {
            return None;
        }
        let methods: Vec<_> = headers
            .get_all("access-control-request-method")
            .iter()
            .collect();
        let valid_method = methods.len() == 1
            && methods[0]
                .to_str()
                .is_ok_and(|m| METHODS.split(", ").any(|allowed| m == allowed));
        let mut requested = Vec::new();
        let valid_headers = headers
            .get_all("access-control-request-headers")
            .iter()
            .all(|value| {
                value.to_str().is_ok_and(|value| {
                    value.split(',').all(|name| {
                        let name = name.trim();
                        if name.parse::<HeaderName>().is_err()
                            || matches!(
                                name.to_ascii_lowercase().as_str(),
                                "host"
                                    | "connection"
                                    | "transfer-encoding"
                                    | "upgrade"
                                    | "cookie"
                                    | "proxy-authenticate"
                            )
                        {
                            return false;
                        }
                        requested.push(name.to_owned());
                        true
                    })
                })
            });
        let mut response = reply(
            if self.origin.is_some() && valid_method && valid_headers {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::FORBIDDEN
            },
            "",
        );
        if response.status() == StatusCode::NO_CONTENT {
            response.headers_mut().insert(
                "access-control-allow-methods",
                HeaderValue::from_static(METHODS),
            );
            if let Ok(value) = requested.join(", ").parse::<HeaderValue>()
                && !requested.is_empty()
            {
                response
                    .headers_mut()
                    .insert("access-control-allow-headers", value);
            }
        }
        vary(response.headers_mut(), "Access-Control-Request-Method");
        vary(response.headers_mut(), "Access-Control-Request-Headers");
        Some(response)
    }
    pub(super) fn apply(&self, response: &mut Response<Body>) {
        if !self.enabled {
            return;
        }
        // The configured policy is authoritative, including for denied origins.
        for name in [
            "access-control-allow-origin",
            "access-control-allow-credentials",
            "access-control-expose-headers",
        ] {
            response.headers_mut().remove(name);
        }
        vary(response.headers_mut(), "Origin");
        if let Some(origin) = &self.origin {
            response.headers_mut().insert(
                "access-control-allow-origin",
                if self.wildcard {
                    HeaderValue::from_static("*")
                } else {
                    origin.clone()
                },
            );
            if self.credentials {
                response.headers_mut().insert(
                    "access-control-allow-credentials",
                    HeaderValue::from_static("true"),
                );
            }
        }
    }
}
fn vary(headers: &mut HeaderMap, name: &'static str) {
    if !headers.get_all("vary").iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim() == "*" || token.trim().eq_ignore_ascii_case(name))
        })
    }) {
        headers.append("vary", HeaderValue::from_static(name));
    }
}
