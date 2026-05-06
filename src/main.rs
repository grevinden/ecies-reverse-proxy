use http_body_util::BodyExt;
mod ecies;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use regex::Regex;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::timeout;
use tracing::{info, warn, error, Level};
use tracing_subscriber::FmtSubscriber;

struct AppState {
    upstream: String,
    secret_key: [u8; 32],
    base64_re: Regex,
    active_connections: AtomicUsize,
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Response<Full<Bytes>> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    info!("Received {} {}", method, uri);

    if method != Method::POST && method != Method::PUT {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::from(Bytes::from("Only POST/PUT allowed")))
            .unwrap();
    }

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to collect body: {}", e);
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::from(Bytes::from(format!("Bad request: {}", e))))
                .unwrap();
        }
    };

    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
    let mut decrypted_count = 0u32;
    let mut modified = body_str.clone();

    for cap in state.base64_re.captures_iter(&body_str) {
        let whole_match = cap.get(0).unwrap().as_str(); // {{...}}
        if let Some(inner) = cap.get(1) { // внутренняя base64 строка
            let candidate = inner.as_str();
            match ecies::decrypt(candidate, &state.secret_key) {
                Ok(plain) => {
                    info!("Successfully decrypted ECIES packet (length: {})", plain.len());
                    modified = modified.replace(whole_match, &plain);
                    decrypted_count += 1;
                }
                Err(e) => warn!("Failed to decrypt candidate: {}", e),
            }
        }
    }

    if decrypted_count > 0 {
        info!("Decrypted {} packets for {}", decrypted_count, uri);
    }

    let upstream_url = format!(
        "{}{}",
        state.upstream,
        uri.path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/")
    );

    let client_req: Request<Full<Bytes>> = match Request::builder()
        .method(method)
        .uri(&upstream_url)
        .header("X-Decrypted-Count", decrypted_count.to_string())
        .body(Full::from(Bytes::from(modified)))
    {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to construct upstream request: {}", e);
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::from(Bytes::from(format!("Internal error: {}", e))))
                .unwrap();
        }
    };

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http();

    match client.request(client_req).await {
        Ok(resp) => {
            info!("Upstream responded with {}", resp.status());
            let (parts, body) = resp.into_parts();
            let body_bytes = match body.collect().await {
                Ok(b) => b.to_bytes(),
                Err(e) => {
                    error!("Failed to collect upstream body: {}", e);
                    return Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .body(Full::from(Bytes::from(format!("Upstream body error: {}", e))))
                        .unwrap();
                }
            };
            Response::from_parts(parts, Full::from(body_bytes))
        }
        Err(e) => {
            error!("Failed to connect to upstream: {}", e);
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Full::from(Bytes::from(format!("Upstream error: {}", e))))
                .unwrap()
        }
    }
}

fn init_tracing() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .compact()
        .finish();
    tracing::subscriber::set_global_default(subscriber).ok();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let private_key_b64 =
        env::var("ECIES_PRIVATE_KEY").expect("ECIES_PRIVATE_KEY environment variable required");
    let secret_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&private_key_b64)
        .expect("Failed to decode private key base64");
    let secret_key: [u8; 32] = secret_bytes
        .as_slice()
        .try_into()
        .expect("Private key must be exactly 32 bytes");

    let upstream =
        env::var("UPSTREAM_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
    let listen_addr =
        env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let base64_re = Regex::new(r"(?mu)[{]{2}([A-Za-z0-9_-]{44,})[}]{2}")?;

    let state = Arc::new(AppState {
        upstream,
        secret_key,
        base64_re,
        active_connections: AtomicUsize::new(0),
    });

    let addr: std::net::SocketAddr = listen_addr.parse()?;
    let listener = TcpListener::bind(&addr).await?;
    info!("ECIES decrypting proxy listening on {}", listen_addr);
    info!("Upstream: {}", state.upstream);

    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Received shutdown signal, starting graceful shutdown...");
        shutdown_clone.notify_waiters();
    });

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        state.active_connections.fetch_add(1, Ordering::SeqCst);
                        let shutdown = shutdown.clone();

                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let state_for_svc = state.clone(); // клонируем Arc для замыкания
                            let svc = service_fn(move |req| {
                                let state = state_for_svc.clone();
                                async { Ok::<_, hyper::Error>(handle_request(req, state).await) }
                            });

                            let mut conn = http1::Builder::new().serve_connection(io, svc);
                            tokio::select! {
                                _ = &mut conn => {},
                                _ = shutdown.notified() => {
                                    let _ = timeout(Duration::from_secs(10), &mut conn).await;
                                }
                            }
                            state.active_connections.fetch_sub(1, Ordering::SeqCst); // исходный state доступен
                        });
                    },
                    Err(e) => {
                        error!("Accept error: {}", e);
                    }
                }
            }
            _ = shutdown.notified() => {
                info!("Stop accepting new connections. Waiting for active connections to finish...");
                drop(listener);
                let wait_duration = Duration::from_secs(30);
                let start = tokio::time::Instant::now();
                loop {
                    let active = state.active_connections.load(Ordering::SeqCst);
                    if active == 0 {
                        info!("All active connections completed");
                        break;
                    }
                    if start.elapsed() > wait_duration {
                        warn!("Timeout waiting for {} active connections, forcing shutdown", active);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                break;
            }
        }
    }

    info!("Server gracefully shut down");
    Ok(())
}