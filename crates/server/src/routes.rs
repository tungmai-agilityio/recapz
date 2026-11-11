use axum::{routing::get, Json, Router};

pub fn router() -> Router {
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .layer(tower_http::trace::TraceLayer::new_for_http())
}
