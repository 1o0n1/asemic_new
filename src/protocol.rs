use crate::state::ObfuscationPattern;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CHUNK_SIZE: usize = 900;
pub const MAX_PACKET_SIZE: usize = 1400;
pub const STARFALL_SIG_SIZE: usize = 4;
pub const MAGIC_KEY: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0xFE, 0xED, 0xCA, 0xFE];

#[derive(Serialize, Deserialize, Debug)]
pub struct AsemicPacket {
    pub msg_id: u32,
    pub chunk_num: u32,
    pub total_chunks: u32,
    pub data: String, // Base64-кодированный чанк данных
}

pub fn xor_cipher(data: &mut [u8], key: &[u8]) {
    if key.is_empty() { return; }
    for (i, byte) in data.iter_mut().enumerate() {
        *byte ^= key[i % key.len()];
    }
}

pub fn create_packet(mut payload: Vec<u8>, key: &[u8], pattern: ObfuscationPattern) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    xor_cipher(&mut payload, key);
    let encrypted_payload = payload;

    let (signature, signature_size) = match pattern {
        ObfuscationPattern::Starfall => {
            let mut hasher = Sha256::new();
            hasher.update(key);
            hasher.update(&encrypted_payload);
            let hash_result = hasher.finalize();
            (hash_result[0..STARFALL_SIG_SIZE].to_vec(), STARFALL_SIG_SIZE)
        }
        ObfuscationPattern::Sunshine => (MAGIC_KEY.to_vec(), MAGIC_KEY.len()),
    };

    let required_size = signature_size + encrypted_payload.len();
    if required_size > MAX_PACKET_SIZE {
        return Vec::new(); // Полезная нагрузка слишком велика
    }
    
    // Дополняем пакет случайными данными до случайного размера
    let final_size = rng.gen_range(required_size..=MAX_PACKET_SIZE);
    let mut final_packet = vec![0u8; final_size];

    final_packet[0..signature_size].copy_from_slice(&signature);
    let payload_end = signature_size + encrypted_payload.len();
    final_packet[signature_size..payload_end].copy_from_slice(&encrypted_payload);
    
    if final_size > payload_end {
        rng.fill_bytes(&mut final_packet[payload_end..]);
    }
    
    final_packet
}

pub fn try_decrypt_packet(packet: &[u8], key: &[u8], pattern: ObfuscationPattern) -> Option<AsemicPacket> {
    let (signature_size, payload_with_padding) = match pattern {
        ObfuscationPattern::Sunshine => {
            if packet.starts_with(&MAGIC_KEY) {
                (MAGIC_KEY.len(), &packet[MAGIC_KEY.len()..])
            } else {
                return None;
            }
        }
        ObfuscationPattern::Starfall => {
            if packet.len() > STARFALL_SIG_SIZE {
                (STARFALL_SIG_SIZE, &packet[STARFALL_SIG_SIZE..])
            } else {
                return None;
            }
        }
    };

    let mut decrypted_payload = payload_with_padding.to_vec();
    xor_cipher(&mut decrypted_payload, key);

    // Первые 4 байта - это длина JSON-пакета
    if let Ok(len_bytes) = decrypted_payload.get(0..4)?.try_into() {
        let json_len = u32::from_be_bytes(len_bytes) as usize;
        let original_data_len = 4 + json_len;

        // Проверяем, что в пакете достаточно данных
        if payload_with_padding.len() >= original_data_len {
            if pattern == ObfuscationPattern::Starfall {
                let received_signature = &packet[0..signature_size];
                let original_encrypted_data = &payload_with_padding[..original_data_len];
                
                let mut hasher = Sha256::new();
                hasher.update(key);
                hasher.update(original_encrypted_data);
                let hash_result = hasher.finalize();
                let expected_signature = &hash_result[0..signature_size];
                
                if received_signature != expected_signature {
                    return None; // Подпись не совпадает
                }
            }

            let json_payload = &decrypted_payload[4..original_data_len];
            return serde_json::from_slice(json_payload).ok();
        }
    }
    None
}