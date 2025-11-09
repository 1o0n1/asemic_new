use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc, Mutex};
use tower_http::services::ServeDir;
use tracing::info;

mod state;
mod protocol;
mod network;
mod processor;
mod web;

use state::{AppState, TransmitCommand, WsNotification};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("asemic_new=info,tower_http=debug")
        .init();

    let exe_path = env::current_exe().expect("Failed to get current executable path");
    let base_dir = exe_path.parent().expect("Executable must be in a directory");

    // --- Путь к статическим файлам ---
    let static_path = base_dir.join("static");
    info!("Expecting static files at: {:?}", static_path);
    if !static_path.exists() {
        tracing::error!("'static' directory not found next to the executable. The web UI will not load.");
        // ВАЖНО: В реальном приложении здесь можно было бы завершить работу или предпринять другие действия
    }
    let serve_dir = ServeDir::new(static_path);

    // --- Путь для загруженных файлов ---
    let downloads_path = base_dir.join("downloads");
    if !downloads_path.exists() {
        info!("'downloads' directory not found. Creating it at: {:?}", downloads_path);
        tokio::fs::create_dir_all(&downloads_path)
            .await
            .expect("Failed to create downloads directory");
    } else {
        info!("Using existing downloads directory at: {:?}", downloads_path);
    }
    
    // --- Инициализация состояния и каналов ---
    let shared_state = Arc::new(Mutex::new(AppState::new(downloads_path)));
    let (packet_tx, packet_rx) = mpsc::channel::<(Vec<u8>, SocketAddr)>(1024);
    let (transmit_tx, transmit_rx) = mpsc::channel::<TransmitCommand>(128);
    let (ws_tx, _) = broadcast::channel::<WsNotification>(128);
    
    // --- UDP сокет ---
    let udp_socket = UdpSocket::bind("0.0.0.0:7070").await.expect("Failed to bind UDP socket");
    info!("UDP socket listening on 0.0.0.0:7070");
    let shared_socket = Arc::new(udp_socket);

    // --- Запуск основных задач ---
    let web_state = Arc::clone(&shared_state);
    let web_task = tokio::spawn(web::run_web_server(web_state, transmit_tx.clone(), ws_tx.clone(), serve_dir));
    
    let receiver_socket = Arc::clone(&shared_socket);
    let receiver_task = tokio::spawn(network::udp_receiver_task(receiver_socket, packet_tx));
    
    let transmitter_socket = Arc::clone(&shared_socket);
    let transmitter_task = tokio::spawn(network::udp_transmitter_task(transmitter_socket, transmit_rx));
    
    let processor_state = Arc::clone(&shared_state);
    let processor_task = tokio::spawn(processor::packet_processor_task(packet_rx, processor_state, ws_tx));

    // --- Ожидание завершения задач ---
    tokio::try_join!(
        web_task,
        receiver_task,
        transmitter_task,
        processor_task
    ).expect("A critical task failed");
}