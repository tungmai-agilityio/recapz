//! The web app, embedded in the binary at build time from `web/dist` (`make release` builds it first).
//! Unknown paths fall back to `index.html` so client-side routes survive a reload; unknown
//! `/api` and `/auth` paths, and missing files with an extension, stay 404s.
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Dist;

/// Which embedded file answers `path` (without the leading slash), or the status to return.
fn resolve<T>(path: &str, get: impl Fn(&str) -> Option<T>) -> Result<(&str, T), StatusCode> {
    if path.starts_with("api/") || path.starts_with("auth/") {
        return Err(StatusCode::NOT_FOUND);
    }
    if !path.is_empty() {
        if let Some(f) = get(path) {
            return Ok((path, f));
        }
        if path.rsplit('/').next().is_some_and(|s| s.contains('.')) {
            return Err(StatusCode::NOT_FOUND); // a missing file, not a client route
        }
    }
    get("index.html")
        .map(|f| ("index.html", f))
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn serve(uri: Uri) -> Response {
    let (name, file) = match resolve(uri.path().trim_start_matches('/'), Dist::get) {
        Ok(hit) => hit,
        Err(status) => return status.into_response(),
    };
    // hashed assets never change; the shell must be re-fetched to pick up a new build
    let cache = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(p: &str) -> Option<&'static str> {
        ["index.html", "assets/app-1.js"]
            .contains(&p)
            .then_some("file")
    }

    #[test]
    fn routes_fall_back_to_the_shell_but_files_and_api_do_not() {
        assert_eq!(resolve("", lookup), Ok(("index.html", "file")));
        assert_eq!(
            resolve("assets/app-1.js", lookup),
            Ok(("assets/app-1.js", "file"))
        );
        assert_eq!(resolve("meetings/42", lookup), Ok(("index.html", "file")));
        assert_eq!(
            resolve("assets/gone.js", lookup),
            Err(StatusCode::NOT_FOUND)
        );
        assert_eq!(resolve("api/nope", lookup), Err(StatusCode::NOT_FOUND));
        assert_eq!(resolve("auth/nope", lookup), Err(StatusCode::NOT_FOUND));
        assert_eq!(
            resolve("meetings/42", |_| None::<()>),
            Err(StatusCode::NOT_FOUND)
        );
    }
}
