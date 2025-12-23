use crate::state::ObfuscationPattern;
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce
};
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Оптимальный размер пакета, чтобы не фрагментировался роутерами (MTU)
// Оставляем запас под UDP заголовок и Nonce.
pub const MAX_PACKET_SIZE: usize = 1350; 
// Размер чанка данных для network.rs (оставляем как было или чуть уменьшаем)
pub const CHUNK_SIZE: usize = 1200; 

#[derive(Serialize, Deserialize, Debug)]
pub struct AsemicPacket {
    pub msg_id: u32,
    pub chunk_num: u32,
    pub total_chunks: u32,
    pub data: String, // Base64-кодированный чанк данных
}

/// Создает 32-байтовый ключ из любой строки пользователя используя SHA-256
fn derive_key(input: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input);
    hasher.finalize().into()
}

pub fn create_packet(payload: Vec<u8>, key: &[u8], _pattern: ObfuscationPattern) -> Vec<u8> {
    let mut rng = rand::thread_rng();

    // 1. Подготовка ключа
    let key_bytes = derive_key(key);
    let cipher = XChaCha20Poly1305::new(&key_bytes.into());

    // 2. Генерация Nonce (24 байта случайности)
    // Это делает каждый пакет уникальным, даже если данные те же.
    let mut nonce = XNonce::default();
    rng.fill_bytes(&mut nonce);

    // 3. Добавление внутреннего паддинга (маскировка размера)
    // Мы добавляем мусор К самим данным ПЕРЕД шифрованием.
    // Так как у нас есть длина JSON в начале payload (от network.rs),
    // при расшифровке мы просто отбросим этот хвост.
    let mut payload_to_encrypt = payload;
    let current_len = payload_to_encrypt.len() + 24 + 16; // payload + nonce + mac tag
    
    // Если пакет меньше максимума, добиваем мусором до случайной длины
    if current_len < MAX_PACKET_SIZE {
        let padding_needed = rng.gen_range(0..=(MAX_PACKET_SIZE - current_len));
        let mut padding = vec![0u8; padding_needed];
        rng.fill_bytes(&mut padding);
        payload_to_encrypt.extend_from_slice(&padding);
    }

    // 4. Шифрование
    // Encrypt возвращает: [EncryptedData + AuthTag]
    let ciphertext = match cipher.encrypt(&nonce, payload_to_encrypt.as_ref()) {
        Ok(ct) => ct,
        Err(_) => return Vec::new(), // Ошибка шифрования
    };

    // 5. Сборка финального пакета: [NONCE] + [CIPHERTEXT]
    // Для внешнего наблюдателя это выглядит как сплошной рандом.
    let mut final_packet = Vec::with_capacity(24 + ciphertext.len());
    final_packet.extend_from_slice(&nonce);
    final_packet.extend_from_slice(&ciphertext);
    
    final_packet
}

pub fn try_decrypt_packet(packet: &[u8], key: &[u8], _pattern: ObfuscationPattern) -> Option<AsemicPacket> {
    // Пакет должен быть хотя бы длиннее Nonce (24 байта) + Tag (16 байт)
    if packet.len() <= 40 { return None; }

    // 1. Подготовка ключа
    let key_bytes = derive_key(key);
    let cipher = XChaCha20Poly1305::new(&key_bytes.into());

    // 2. Разбор пакета
    let nonce = XNonce::from_slice(&packet[0..24]);
    let ciphertext = &packet[24..];

    // 3. Попытка расшифровки
    // Если ключ не тот, или пакет битый, или это просто шум интернета -> вернет Err
    match cipher.decrypt(nonce, ciphertext) {
        Ok(decrypted_data) => {
            // Данные расшифрованы! Теперь нужно отделить полезную нагрузку от паддинга.
            // Структура payload из network.rs: [4 bytes Length][JSON][Padding...]
            
            if decrypted_data.len() < 4 { return None; }
            
            // Читаем длину полезного JSON блока
            if let Ok(len_bytes) = decrypted_data[0..4].try_into() {
                let json_len = u32::from_be_bytes(len_bytes) as usize;
                
                // Проверяем, что длина адекватна
                if decrypted_data.len() < 4 + json_len { return None; }
                
                // Вырезаем чистый JSON, игнорируя хвост с мусором
                let json_slice = &decrypted_data[4..4+json_len];
                
                return serde_json::from_slice(json_slice).ok();
            }
            None
        }
        Err(_) => None, // Не удалось расшифровать (чужой пакет или шум)
    }
}