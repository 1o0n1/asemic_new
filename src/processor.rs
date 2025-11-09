use crate::state::{
    FileContent, MessageContent, SharedState, WsNotification, DecryptedMessage, ObfuscationPattern};
use crate::protocol;
use base64::{engine::general_purpose, Engine};
use std::net::SocketAddr;
use tokio::sync::{mpsc, broadcast};
use tracing::{info, warn, error, debug};
use uuid::Uuid;

pub async fn packet_processor_task(
    mut packet_receiver: mpsc::Receiver<(Vec<u8>, SocketAddr)>,
    state: SharedState,
    ws_tx: broadcast::Sender<WsNotification>,
) {
    info!("Packet processor task started.");
    // Паттерны, которые мы будем пробовать при дешифровке
    let patterns_to_try = [ObfuscationPattern::Starfall, ObfuscationPattern::Sunshine];

    while let Some((packet, sender)) = packet_receiver.recv().await {
        let mut decrypted_successfully = false;
        
        // Блокируем состояние один раз перед циклом для получения ключей
        let keys = {
            let mut state_guard = state.lock().await;
            state_guard.stats.packets_received += 1;
            // Отправляем обновление статистики всем клиентам
            ws_tx.send(WsNotification::StatsUpdate(state_guard.stats)).ok();
            state_guard.keys.clone()
        };

        // Перебираем все известные ключи и паттерны, чтобы попытаться расшифровать пакет
        'decryption_loop: for &pattern in &patterns_to_try {
            for key in &keys {
                if let Some(asemic_packet) = protocol::try_decrypt_packet(&packet, key.as_bytes(), pattern) {
                    decrypted_successfully = true;
                    debug!("Decrypted a packet from {} with key '{}' and pattern {:?}", sender, key, pattern);
                    
                    // Декодируем данные чанка из Base64
                    let chunk_data = match general_purpose::STANDARD.decode(&asemic_packet.data) {
                        Ok(data) => data,
                        Err(e) => {
                            warn!("Failed to decode Base64 chunk from {}: {}", sender, e);
                            continue; // Пропускаем этот чанк, он поврежден
                        }
                    };

                    let mut state_guard = state.lock().await;
                    let session_key = (sender, asemic_packet.msg_id);
                    
                    // Получаем или создаем буфер для сборки сообщения
                    let session_chunks = state_guard.reassembly_buffer.entry(session_key).or_default();
                    session_chunks.insert(asemic_packet.chunk_num, chunk_data);
                    
                    // Проверяем, все ли части сообщения получены
                    if session_chunks.len() as u32 == asemic_packet.total_chunks {
                        info!("Full message {} from {} assembled ({} chunks).", asemic_packet.msg_id, sender, asemic_packet.total_chunks);
                        let mut full_message_bytes = Vec::new();
                        for i in 0..asemic_packet.total_chunks {
                            if let Some(chunk) = session_chunks.get(&i) {
                                full_message_bytes.extend_from_slice(chunk);
                            } else {
                                warn!("Missing chunk #{} for message {}. Aborting assembly.", i, asemic_packet.msg_id);
                                // Удаляем неполное сообщение из буфера
                                state_guard.reassembly_buffer.remove(&session_key);
                                break;
                            }
                        }
                        // Удаляем сообщение из буфера после успешной сборки
                        state_guard.reassembly_buffer.remove(&session_key);
                        
                        // --- КЛЮЧЕВАЯ ЛОГИКА ---
                        // Теперь, когда у нас есть полный набор байт, мы десериализуем его обратно в MessageContent.
                        match serde_json::from_slice::<MessageContent>(&full_message_bytes) {
                            Ok(content) => {
                                // Обрабатываем контент: если это файл, сохраняем его
                                let final_content = match content {
                                    MessageContent::File(file_content) => {
                                        let file_id = Uuid::new_v4();
                                        // Сохраняем файл в фоновой задаче, чтобы не блокировать обработку пакетов
                                        let downloads_path = state_guard.downloads_path.clone();
                                        let filename_for_task = file_content.filename.clone();
                                        let file_data_for_task = file_content.data.clone();

                                        tokio::spawn(async move {
                                            let file_path = downloads_path.join(&filename_for_task);
                                            info!("Saving received file to {:?}", &file_path);
                                            if let Err(e) = tokio::fs::write(&file_path, &file_data_for_task).await {
                                                error!("Failed to save file {:?}: {}", file_path, e);
                                            }
                                        });

                                        // Для отображения в UI, мы не хотим отправлять все данные файла.
                                        // Отправляем только информацию о нем.
                                        let content_for_ui = MessageContent::File(FileContent {
                                            filename: file_content.filename.clone(),
                                            data: Vec::new(), // Очищаем данные для отправки в UI
                                            id: Some(file_id),
                                        });
                                        // Сохраняем файл в памяти для возможности скачивания
                                        state_guard.received_files.insert(file_id, (file_content.filename, file_content.data));
                                        
                                        content_for_ui
                                    },
                                    text_content => text_content,
                                };
                                
                                let message = DecryptedMessage {
                                    id: Uuid::new_v4(),
                                    timestamp: chrono::Utc::now(),
                                    sender,
                                    content: final_content,
                                    decrypted_with_key: key.clone(),
                                    decrypted_with_pattern: pattern,
                                };
                                
                                state_guard.messages.push(message.clone());
                                state_guard.stats.messages_decrypted += 1;
                                // Уведомляем UI о новом сообщении и обновлении статистики
                                ws_tx.send(WsNotification::NewMessage(message)).ok();
                                ws_tx.send(WsNotification::StatsUpdate(state_guard.stats)).ok();

                            },
                            Err(e) => {
                                warn!("Failed to deserialize assembled message content from {}: {}. Raw bytes len: {}", sender, e, full_message_bytes.len());
                            }
                        }
                    }
                    // Если пакет успешно расшифрован, прекращаем перебор ключей
                    break 'decryption_loop;
                }
            }
        }
        // Если ни один ключ/паттерн не подошел, считаем пакет шумом
        if !decrypted_successfully {
            debug!("Received a noise packet of size {} from {}", packet.len(), sender);
            ws_tx.send(WsNotification::NoisePacket { sender, size: packet.len() }).ok();
        }
    }
}