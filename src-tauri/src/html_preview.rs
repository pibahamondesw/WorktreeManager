//! Serves chat HTML previews from their own origin so their scripts run under the preview CSP
//! instead of the app's. Tauri treats every registered scheme as a local origin for IPC, so the
//! isolation from the app relies on the frame never getting `allow-same-origin` and on the main
//! window never navigating here: only the main frame receives Tauri's invoke key. WebView2 injects
//! it into every frame, so Windows keeps static previews and never loads this protocol.

use tauri::http::{header, Method, Request, Response, StatusCode};

pub const SCHEME: &str = "wtm-preview";

const PREVIEW_CSP: &str = "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline' https://cdn.jsdelivr.net; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; form-action 'none'; base-uri 'none'";

/// The page travels base64url-encoded in the URL fragment, which never reaches this handler, so
/// the request stays small whatever the page size. Escape keys the page leaves unhandled, checked
/// once every page listener ran, go to the app, which only accepts them from its own frame, so
/// full screen closes while the page has focus.
const BOOTSTRAP: &str = r#"<!doctype html><meta charset="utf-8"><script>
addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || event.isComposing) return;
  setTimeout(() => event.defaultPrevented || parent.postMessage("wtm-preview:escape", "*"));
});
const base64 = location.hash.slice(1).replace(/-/g, "+").replace(/_/g, "/");
const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
document.write(new TextDecoder().decode(bytes));
</script>"#;

pub fn respond(request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let response = Response::builder()
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer");
    let response = if request.method() == Method::GET {
        response
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_SECURITY_POLICY, PREVIEW_CSP)
            .body(BOOTSTRAP.as_bytes().to_vec())
    } else {
        response
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Vec::new())
    };
    response.expect("static response parts are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: Method, uri: &str) -> Request<Vec<u8>> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Vec::new())
            .unwrap()
    }

    fn header_value(response: &Response<Vec<u8>>, name: header::HeaderName) -> &str {
        response.headers()[name].to_str().unwrap()
    }

    #[test]
    fn serves_the_bootstrap_with_the_preview_policy_for_any_path() {
        for uri in [
            "wtm-preview://localhost/",
            "http://wtm-preview.localhost/anything?query=1",
        ] {
            let response = respond(&request(Method::GET, uri));

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.body(), BOOTSTRAP.as_bytes());
            assert_eq!(
                header_value(&response, header::CONTENT_TYPE),
                "text/html; charset=utf-8"
            );
            assert_eq!(header_value(&response, header::CACHE_CONTROL), "no-store");
            assert_eq!(
                header_value(&response, header::X_CONTENT_TYPE_OPTIONS),
                "nosniff"
            );
        }
    }

    #[test]
    fn preview_policy_keeps_the_page_opaque_and_offline() {
        let response = respond(&request(Method::GET, "wtm-preview://localhost/"));

        let directives: Vec<&str> = header_value(&response, header::CONTENT_SECURITY_POLICY)
            .split("; ")
            .collect();
        assert_eq!(
            directives,
            [
                "sandbox allow-scripts",
                "default-src 'none'",
                "script-src 'unsafe-inline' https://cdn.jsdelivr.net",
                "style-src 'unsafe-inline'",
                "img-src data: blob:",
                "font-src data:",
                "media-src data: blob:",
                "form-action 'none'",
                "base-uri 'none'",
            ]
        );
    }

    #[test]
    fn bootstrap_forwards_unhandled_escape_keys_to_the_app() {
        assert!(BOOTSTRAP.contains(
            r#"setTimeout(() => event.defaultPrevented || parent.postMessage("wtm-preview:escape", "*"))"#
        ));
    }

    #[test]
    fn rejects_methods_other_than_get() {
        let response = respond(&request(Method::POST, "wtm-preview://localhost/"));

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(response.body().is_empty());
        assert!(!response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY));
    }

    fn app_csp_directives() -> Vec<String> {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        config["app"]["security"]["csp"]
            .as_str()
            .unwrap()
            .split("; ")
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn app_policy_frames_only_the_preview_protocol_and_adds_no_scripts() {
        let directives = app_csp_directives();

        assert!(
            directives.contains(&"frame-src wtm-preview: http://wtm-preview.localhost".to_owned())
        );
        assert!(!directives.iter().any(|d| d.starts_with("script-src")));
    }

    #[test]
    fn capabilities_stay_on_the_main_webview_without_remote_origins() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();

        assert_eq!(capability["webviews"], serde_json::json!(["main"]));
        assert!(capability.get("remote").is_none());
    }
}
