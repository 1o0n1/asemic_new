use crate::protocol;
// ИСПРАВЛЕНИЕ: Добавлены `ObfuscationPattern` и `MessageContent` в импорты.
use crate::state::{NoiseLevel, TransmitCommand, ObfuscationPattern};
use base64::{engine::general_purpose, Engine};
use rand::Rng;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub async fn udp_transmitter_task(
    socket: Arc<UdpSocket>,
    mut command_receiver: mpsc::Receiver<TransmitCommand>,
) {
    info!("UDP transmitter task started.");
    
    let mut noise_interval = tokio::time::interval(Duration::from_secs(u64::MAX));
    let mut noise_level = NoiseLevel::Off;
    let mut last_target: Option<SocketAddr> = None;
    let mut last_key: Option<String> = None;
    // Теперь эта строка корректна, так как тип импортирован
    let mut last_pattern = ObfuscationPattern::Starfall; 

    loop {
        tokio::select! {
            Some(command) = command_receiver.recv() => {
                match command {
                    TransmitCommand::SendMessage { target_addr, key, pattern, content } => {
                        info!("Transmitting message to {} using pattern {:?}", target_addr, pattern);
                        
                        last_target = Some(target_addr);
                        last_key = Some(key.clone());
                        last_pattern = pattern;

                        let data_to_chunk = match serde_json::to_vec(&content) {
                            Ok(data) => data,
                            Err(e) => {
                                error!("Failed to serialize message content to JSON: {}", e);
                                continue;
                            }
                        };
                        
                        let msg_id: u32 = rand::thread_rng().gen();
                        let chunks: Vec<&[u8]> = data_to_chunk.chunks(protocol::CHUNK_SIZE).collect();
                        let total_chunks = chunks.len() as u32;

                        info!("Splitting content ({} bytes) into {} chunks for message ID {}.", data_to_chunk.len(), total_chunks, msg_id);

                        for (i, chunk) in chunks.into_iter().enumerate() {
                            let data_b64 = general_purpose::STANDARD.encode(chunk);
                            
                            let asemic_packet = protocol::AsemicPacket {
                                msg_id,
                                chunk_num: i as u32,
                                total_chunks,
                                data: data_b64,
                            };
                            
                            let json_payload = serde_json::to_vec(&asemic_packet).unwrap();
                            
                            let mut plaintext_payload = Vec::with_capacity(4 + json_payload.len());
                            plaintext_payload.extend_from_slice(&(json_payload.len() as u32).to_be_bytes());
                            plaintext_payload.extend_from_slice(&json_payload);
                            
                            let final_packet = protocol::create_packet(plaintext_payload, key.as_bytes(), pattern);

                            if final_packet.is_empty() {
                                error!("Generated packet for chunk {}/{} is too large and was dropped.", i + 1, total_chunks);
                                continue;
                            }

                            if let Err(e) = socket.send_to(&final_packet, target_addr).await {
                                error!("Failed to send data packet to {}: {}", target_addr, e);
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        info!("Finished sending all {} chunks for message {}", total_chunks, msg_id);
                    }
                    TransmitCommand::SetNoiseLevel(level) => {
                        noise_level = level;
                        let duration = match level {
                            NoiseLevel::Off => Duration::from_secs(u64::MAX),
                            NoiseLevel::Slow => Duration::from_millis(2000),
                            NoiseLevel::Medium => Duration::from_millis(500),
                            NoiseLevel::Fast => Duration::from_millis(100),
                        };
                        noise_interval = tokio::time::interval(duration);
                        info!("Noise interval updated to {:?}", duration);
                    }
                }
            }
            _ = noise_interval.tick(), if noise_level != NoiseLevel::Off => {
                if let (Some(target), Some(key)) = (last_target, last_key.as_ref()) {
                    let mut noise_payload: Vec<u8> = vec![0; rand::thread_rng().gen_range(50..200)];
                    rand::thread_rng().fill(&mut noise_payload[..]);

                    let noise_packet = protocol::create_packet(noise_payload, key.as_bytes(), last_pattern);

                     if let Err(e) = socket.send_to(&noise_packet, target).await {
                        error!("Failed to send noise packet: {}", e);
                    }
                } else {
                     warn!("Noise tick: No target or key available to send noise.");
                }
            }
        }
    }
}

pub async fn udp_receiver_task(
    socket: Arc<UdpSocket>,
    packet_sender: mpsc::Sender<(Vec<u8>, SocketAddr)>,
) {
    info!("UDP receiver task started.");
    let mut buf = vec![0u8; 2048];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, sender_addr)) => {
                let packet_data = buf[..len].to_vec();
                if let Err(e) = packet_sender.send((packet_data, sender_addr)).await {
                    error!("Failed to send packet to processor: {}", e);
                }
            }
            Err(e) => {
                error!("Error receiving from UDP socket: {}", e);
            }
        }
    }
}