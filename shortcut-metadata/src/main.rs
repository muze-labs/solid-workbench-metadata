mod discovery;
mod outbound;

use std::{
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, Method, StatusCode, header},
    routing::{get, post},
};
use moka::sync::Cache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;
use url::Url;

#[derive(Clone)]
struct Service {
    icons: Cache<String, Vec<u8>>,
    slots: Arc<Semaphore>,
    budget: Arc<Mutex<(Instant, u32)>>,
}

impl Service {
    fn take_request(&self) -> Result<(), StatusCode> {
        let mut budget = self.budget.lock().unwrap();
        if budget.0.elapsed() >= Duration::from_secs(60) {
            *budget = (Instant::now(), 0);
        }
        if budget.1 >= 120 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        budget.1 += 1;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}

async fn metadata(
    State(service): State<Service>,
    Json(request): Json<Request>,
) -> Result<([(header::HeaderName, &'static str); 1], Json<Metadata>), StatusCode> {
    let url = Url::parse(&request.url)
        .ok()
        .and_then(outbound::checked_url)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let _slot = service
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    service.take_request()?;
    let (title, icon) = discovery::discover(url).await;
    let icon_url = icon.map(|icon| {
        let id = format!("{:x}", Sha256::digest(&icon));
        service.icons.insert(id.clone(), icon);
        format!("/shortcut-metadata/icons/{id}.png")
    });
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(Metadata { title, icon_url }),
    ))
}

async fn icon(
    State(service): State<Service>,
    Path(file): Path<String>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), StatusCode> {
    let id = file.strip_suffix(".png").ok_or(StatusCode::NOT_FOUND)?;
    let png = service.icons.get(id).ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        png,
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = Service {
        icons: Cache::builder()
            .max_capacity(16 * 1024 * 1024)
            .weigher(|_: &String, bytes: &Vec<u8>| bytes.len() as u32 + 256)
            .build(),
        slots: Arc::new(Semaphore::new(8)),
        budget: Arc::new(Mutex::new((Instant::now(), 0))),
    };
    let mut app = Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/shortcut-metadata", post(metadata))
        .route("/shortcut-metadata/icons/{file}", get(icon))
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(state);
    if let Ok(origin) = env::var("WORKBENCH_ORIGIN") {
        let parsed = Url::parse(&origin)?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.origin().ascii_serialization() != origin
        {
            return Err(
                "WORKBENCH_ORIGIN must be a single HTTP(S) origin without a trailing slash".into(),
            );
        }
        app = app.layer(
            CorsLayer::new()
                .allow_origin(origin.parse::<HeaderValue>()?)
                .allow_methods([Method::POST, Method::GET])
                .allow_headers([header::CONTENT_TYPE]),
        );
    }
    let address = env::var("METADATA_BIND").unwrap_or_else(|_| "127.0.0.1:4175".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!("Shortcut metadata listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
