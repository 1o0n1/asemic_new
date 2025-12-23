// ИСПРАВЛЕНИЕ: Теперь импортируем всё необходимое из state.rs, где оно централизованно определено.
use crate::state::{
    SharedState, TransmitCommand, WsNotification, AddKeyPayload,
    SendMessagePayload, SetNoisePayload
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, delete},
    Json, Router,
};
// Убрали serde::Deserialize, так как структуры теперь в state.rs
use tokio::net::lookup_host;
use tokio::sync::{broadcast, mpsc};
use tower_http::services::ServeDir;
use tracing::{info, warn};
use uuid::Uuid;
use std::sync::Arc;

// ИСПРАВЛЕНИЕ: Структуры `AddKeyPayload`, `SendMessagePayload`, `SetNoisePayload` удалены отсюда,
// так как они перенесены в `state.rs`

type WebState = (
    SharedState,
    mpsc::Sender<TransmitCommand>,
    broadcast::Sender<WsNotification>,
);

pub async fn run_web_server(
    state: SharedState,
    transmit_sender: mpsc::Sender<TransmitCommand>,
    ws_tx: broadcast::Sender<WsNotification>,
    serve_dir: ServeDir,
) {
    let app_state: WebState = (state, transmit_sender, ws_tx);

    let app = Router::new()
        .nest_service("/static", serve_dir)
        .route("/", get(serve_index))
        .route("/ws", get(websocket_handler))
        .route("/keys", post(add_key_handler))
        .route("/keys", delete(remove_key_handler))
        .route("/send", post(send_message_handler))
        .route("/download/:file_id", get(download_file_handler))
        .route("/config/noise", post(set_noise_handler))
        .with_state(Arc::new(app_state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    info!("Web server listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}


async fn serve_index() -> impl IntoResponse {
    axum::response::Html(tokio::fs::read_to_string("static/index.html").await.unwrap_or_else(|e| {
        warn!("Failed to read index.html: {}", e);
        "<html><body><h1>Error</h1><p>Could not load frontend. Make sure 'static/index.html' exists.</p></body></html>".to_string()
    }))
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<WebState>) {
    let (shared_state, _, ws_tx) = &*state;
    let mut ws_rx = ws_tx.subscribe();

    let initial_state;
    {
        let state_guard = shared_state.lock().await;
        initial_state = WsNotification::FullState {
            keys: state_guard.keys.clone(),
            messages: state_guard.messages.clone(),
            stats: state_guard.stats,
        };
    }

    if let Ok(json) = serde_json::to_string(&initial_state) {
        if socket.send(Message::Text(json)).await.is_err() {
            return;
        }
    }

    loop {
        match ws_rx.recv().await {
            Ok(notification) => {
                if let Ok(json) = serde_json::to_string(&notification) {
                    if socket.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
            Err(e) => {
                warn!("WebSocket broadcast receiver error: {}; disconnecting client.", e);
                break;
            }
        }
    }
}

async fn add_key_handler(
    State(state): State<Arc<WebState>>,
    Json(payload): Json<AddKeyPayload>,
) -> impl IntoResponse {
    let (shared_state, _, ws_tx) = &*state;
    let mut state_guard = shared_state.lock().await;
    if !state_guard.keys.contains(&payload.key) && !payload.key.is_empty() {
        state_guard.keys.push(payload.key.clone());
        info!("Added new key: {}", payload.key);
    }
    let keys = state_guard.keys.clone();
    ws_tx.send(WsNotification::KeyUpdate(keys)).ok();
    StatusCode::OK
}

async fn remove_key_handler(
    State(state): State<Arc<WebState>>,
    Json(payload): Json<AddKeyPayload>,
) -> impl IntoResponse {
    let (shared_state, _, ws_tx) = &*state;
    let mut state_guard = shared_state.lock().await;
    state_guard.keys.retain(|k| k != &payload.key);
    info!("Removed key: {}", payload.key);
    let keys = state_guard.keys.clone();
    ws_tx.send(WsNotification::KeyUpdate(keys)).ok();
    StatusCode::OK
}

async fn send_message_handler(
    State(state): State<Arc<WebState>>,
    Json(payload): Json<SendMessagePayload>,
) -> impl IntoResponse {
    let (_, transmit_sender, _) = &*state;
    
    match lookup_host(&payload.target_addr).await {
        Ok(mut addresses) => {
            if let Some(target_addr) = addresses.next() {
                let command = TransmitCommand::SendMessage {
                    target_addr,
                    key: payload.key,
                    pattern: payload.pattern,
                    content: payload.content,
                };
                if transmit_sender.send(command).await.is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to queue message").into_response();
                }
                (StatusCode::OK, "Message queued").into_response()
            } else {
                (StatusCode::BAD_REQUEST, "Domain name could not be resolved").into_response()
            }
        }
        Err(e) => {
            warn!("Failed to resolve host '{}': {}", payload.target_addr, e);
            (StatusCode::BAD_REQUEST, "Invalid target address or domain").into_response()
        }
    }
}

async fn download_file_handler(
    State(state): State<Arc<WebState>>,
    Path(file_id): Path<Uuid>,
) -> impl IntoResponse {
    let (shared_state, _, _) = &*state;
    let state_guard = shared_state.lock().await;
    if let Some((filename, data)) = state_guard.received_files.get(&file_id) {
        let headers = [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ];
        Ok((headers, data.clone()))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn set_noise_handler(
    State(state): State<Arc<WebState>>,
    Json(payload): Json<SetNoisePayload>,
) -> Response {
    let (_, transmit_sender, _) = &*state;
    info!("Setting noise level to: {:?}", payload.level);
    let command = TransmitCommand::SetNoiseLevel(payload.level);
    if transmit_sender.send(command).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to set noise level").into_response();
    }
    StatusCode::OK.into_response()
}