document.addEventListener('DOMContentLoaded', () => {
    // --- Элементы UI ---
    const wsStatus = document.getElementById('ws-status');
    const addKeyForm = document.getElementById('add-key-form');
    const keyInput = document.getElementById('key-input');
    const keyList = document.getElementById('key-list');
    const sendMessageForm = document.getElementById('send-message-form');
    const targetAddrInput = document.getElementById('target-addr');
    const sendKeySelect = document.getElementById('send-key');
    const sendPatternSelect = document.getElementById('send-pattern');
    const messageTextInput = document.getElementById('message-text');
    const fileInput = document.getElementById('file-input');
    const fileNameDisplay = document.getElementById('file-name-display');
    const messageFeed = document.getElementById('message-feed');
    const trafficFeed = document.getElementById('traffic-feed');
    const currentKeyDisplay = document.getElementById('current-key');
    const noiseLevelRadios = document.querySelectorAll('input[name="noise"]');
    
    // --- Элементы статистики ---
    const statSent = document.getElementById('stat-sent');
    const statNoiseSent = document.getElementById('stat-noise-sent');
    const statReceived = document.getElementById('stat-received');
    const statDecrypted = document.getElementById('stat-decrypted');

    function connectWebSocket() {
        const ws = new WebSocket(`ws://${window.location.host}/ws`);

        ws.onopen = () => {
            wsStatus.textContent = 'Connected';
            wsStatus.className = 'status-connected';
        };

        ws.onclose = () => {
            wsStatus.textContent = 'Disconnected. Retrying...';
            wsStatus.className = 'status-disconnected';
            setTimeout(connectWebSocket, 3000);
        };

        ws.onerror = (error) => {
            console.error('WebSocket Error:', error);
            ws.close();
        };

        ws.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                handleWsMessage(data);
            } catch (e) {
                console.error("Failed to parse WebSocket message:", event.data, e);
            }
        };
    }

    function handleWsMessage(data) {
        switch (data.event) {
            case 'FullState':
                renderKeys(data.data.keys);
                renderMessages(data.data.messages);
                updateStats(data.data.stats);
                break;
            case 'NewMessage':
                renderMessage(data.data, true);
                clearFeedPlaceholder(messageFeed);
                break;
            case 'NoisePacket':
                renderTraffic(data.data);
                 clearFeedPlaceholder(trafficFeed);
                break;
            case 'KeyUpdate':
                renderKeys(data.data);
                break;
            case 'StatsUpdate':
                updateStats(data.data);
                break;
        }
    }

    function renderKeys(keys) {
        keyList.innerHTML = '';
        const currentSelectedKey = sendKeySelect.value;
        sendKeySelect.innerHTML = '<option value="" disabled selected>--Select a key--</option>';
        
        if (keys.length === 0) {
            const li = document.createElement('li');
            li.textContent = 'No keys added.';
            li.className = 'no-keys';
            keyList.appendChild(li);
        } else {
            keys.forEach(key => {
                const li = document.createElement('li');
                li.textContent = key;
                const deleteBtn = document.createElement('button');
                deleteBtn.textContent = '✖';
                deleteBtn.className = 'delete-key';
                deleteBtn.title = `Remove key ${key}`;
                deleteBtn.onclick = () => removeKey(key);
                li.appendChild(deleteBtn);
                keyList.appendChild(li);

                const option = document.createElement('option');
                option.value = key;
                option.textContent = key;
                sendKeySelect.appendChild(option);
            });
        }
        
        // Восстанавливаем выбор, если ключ все еще существует
        if (keys.includes(currentSelectedKey)) {
            sendKeySelect.value = currentSelectedKey;
        }
        updateCurrentKeyDisplay();
    }
    
    function updateCurrentKeyDisplay() {
        currentKeyDisplay.textContent = sendKeySelect.value || 'None';
    }

    function renderMessages(messages) {
        messageFeed.innerHTML = '';
        if (messages.length > 0) {
            messages.forEach(msg => renderMessage(msg, false));
        } else {
            messageFeed.innerHTML = '<div class="feed-placeholder">Waiting for messages...</div>';
        }
    }

    function renderMessage(msg, prepend = true) {
        const item = document.createElement('div');
        item.className = 'feed-item message';

        const timestamp = new Date(msg.timestamp).toLocaleTimeString();
        let contentHtml = '';
        const content = msg.content.payload;

        if (msg.content.type === 'Text') {
            contentHtml = `<div class="message-content">${escapeHtml(content)}</div>`;
        } else if (msg.content.type === 'File') {
            contentHtml = `
                <div class="message-content file-attachment">
                    📎 File: <strong>${escapeHtml(content.filename)}</strong>
                    <a href="/download/${content.id}" target="_blank" class="download-link">Download</a>
                </div>
            `;
        }

        item.innerHTML = `
            <div class="message-meta">
                <span class="timestamp">[${timestamp}]</span> 
                From <span class="message-sender">${msg.sender}</span> 
                (key: <span class="key-used">${escapeHtml(msg.decrypted_with_key)}</span>, 
                pattern: <span class="pattern-used">${msg.decrypted_with_pattern}</span>)
            </div>
            ${contentHtml}
        `;

        if (prepend) {
            messageFeed.insertBefore(item, messageFeed.firstChild);
        } else {
            messageFeed.appendChild(item);
        }
    }

    function renderTraffic(packet) {
        const item = document.createElement('div');
        item.className = 'feed-item noise';
        const timestamp = new Date().toLocaleTimeString();
        item.innerHTML = `<span class="timestamp">[${timestamp}]</span> RECV from ${packet.sender} | ${packet.size} bytes | <span class="noise-label">Noise/Undecrypted</span>`;
        
        trafficFeed.insertBefore(item, trafficFeed.firstChild);
        // Ограничиваем количество записей в ленте, чтобы не перегружать браузер
        while (trafficFeed.children.length > 200) {
            trafficFeed.removeChild(trafficFeed.lastChild);
        }
    }
    
    function updateStats(stats) {
        statSent.textContent = stats.packets_sent;
        statNoiseSent.textContent = stats.noise_packets_sent;
        statReceived.textContent = stats.packets_received;
        statDecrypted.textContent = stats.messages_decrypted;
    }

    // --- Функции для взаимодействия с API ---

    async function apiFetch(endpoint, method, body) {
        try {
            const response = await fetch(endpoint, {
                method: method,
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body)
            });
            if (!response.ok) {
                const errorText = await response.text();
                throw new Error(`API Error (${response.status}): ${errorText}`);
            }
            return response;
        } catch (error) {
            console.error(`Fetch failed for ${method} ${endpoint}:`, error);
            alert(error.message); // Показываем ошибку пользователю
            return null;
        }
    }

    async function addKey(key) {
        await apiFetch('/keys', 'POST', { key });
    }

    async function removeKey(key) {
        await apiFetch('/keys', 'DELETE', { key });
    }

    async function setNoiseLevel(level) {
        await apiFetch('/config/noise', 'POST', { level });
    }

    async function sendMessage(payload) {
        return await apiFetch('/send', 'POST', payload);
    }
    
    // --- Обработчики событий ---

    addKeyForm.addEventListener('submit', (e) => {
        e.preventDefault();
        const key = keyInput.value.trim();
        if (key) {
            addKey(key);
            keyInput.value = '';
        }
    });

    sendMessageForm.addEventListener('submit', async (e) => {
        e.preventDefault();
        const targetAddr = targetAddrInput.value.trim();
        const key = sendKeySelect.value;
        const pattern = sendPatternSelect.value;
        const text = messageTextInput.value.trim();
        const file = fileInput.files[0];

        if (!targetAddr || !key || (!text && !file)) {
            alert('Please provide Target, Key, and either a message or a file.');
            return;
        }

        let content;

        if (file) {
            // Если выбран файл, используем его
            try {
                const base64Data = await fileToBase64(file);
                content = {
                    type: 'File',
                    payload: { filename: file.name, data: base64Data }
                };
            } catch (error) {
                alert(`Error reading file: ${error.message}`);
                return;
            }
        } else {
            // Иначе используем текст
            content = { type: 'Text', payload: text };
        }
        
        const payload = {
            target_addr: targetAddr,
            key: key,
            pattern: pattern, // КЛЮЧЕВОЕ ИСПРАВЛЕНИЕ: Добавляем pattern в запрос
            content: content
        };

        const response = await sendMessage(payload);
        
        if (response) {
            // Очищаем поля после успешной отправки
            messageTextInput.value = '';
            fileInput.value = '';
            fileNameDisplay.textContent = '';
        }
    });
    
    fileInput.addEventListener('change', () => {
        if (fileInput.files.length > 0) {
            fileNameDisplay.textContent = `Selected: ${fileInput.files[0].name}`;
            messageTextInput.value = ''; // Очищаем текстовое поле, т.к. файл в приоритете
        } else {
            fileNameDisplay.textContent = '';
        }
    });
    
    sendKeySelect.addEventListener('change', updateCurrentKeyDisplay);

    noiseLevelRadios.forEach(radio => {
        radio.addEventListener('change', (e) => {
            setNoiseLevel(e.target.value);
        });
    });

    // --- Утилиты ---

    function fileToBase64(file) {
        return new Promise((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => {
                // reader.result это ArrayBuffer, его нужно конвертировать в строку base64
                // Вырезаем "data:*/*;base64,"
                const base64String = reader.result.split(',')[1];
                resolve(base64String);
            };
            reader.onerror = error => reject(error);
            reader.readAsDataURL(file); // Этот метод сразу кодирует в base64
        });
    }
    
    function clearFeedPlaceholder(feedElement) {
        const placeholder = feedElement.querySelector('.feed-placeholder');
        if (placeholder) {
            placeholder.remove();
        }
    }

    function escapeHtml(unsafe) {
        return unsafe
             .replace(/&/g, "&amp;")
             .replace(/</g, "&lt;")
             .replace(/>/g, "&gt;")
             .replace(/"/g, "&quot;")
             .replace(/'/g, "&#039;");
    }

    // --- Запуск ---
    connectWebSocket();
});