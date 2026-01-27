let currentConfig = null;
let isSettingsOpen = false;
let retentionDays = 3; // Default
let monitorData = [];
let eventSource = null;

const DEFAULT_PORTS = {
    'TCP': 22,
    'DNS': 53,
    'HTTP': 80,
    'HTTPS': 443
};

// Init config first to get retention days
async function init() {
    await loadConfig(false); // Load config silently
    retentionDays = currentConfig?.data_retention_days || 3;
    // Update dropdown "Retention Policy" label
    const retentionOpt = document.querySelector('option[value="retention"]');
    if (retentionOpt) retentionOpt.text = `Retention Policy (${retentionDays} Days)`;
    
    startDashboardUpdates();
}

function toggleSettings() {
    isSettingsOpen = !isSettingsOpen;
    const dash = document.getElementById('view-dashboard');
    const settings = document.getElementById('view-settings');
    const btn = document.getElementById('btn-settings');
    const timeSelect = document.getElementById('time-range');

    if (isSettingsOpen) {
        dash.classList.add('hidden');
        settings.classList.remove('hidden');
        btn.classList.add('active');
        timeSelect.disabled = true;
        stopDashboardUpdates();
        loadConfig(true);
    } else {
        settings.classList.add('hidden');
        dash.classList.remove('hidden');
        btn.classList.remove('active');
        timeSelect.disabled = false;
        startDashboardUpdates();
    }
}

function updateTimeRange() {
    renderDashboard(monitorData); // Refresh view with current data
}

// --- Dashboard Logic ---

function renderDashboard(data) {
    const list = document.getElementById('monitor-list');
    list.innerHTML = '';
    
    // Determine time range limit (in count of records, approx)
    const rangeVal = document.getElementById('time-range').value;
    let maxRecords = 60; // Default view width
    
    // 1 record = 10 seconds.
    // If "retention", we use all available records.
    // If number, it's number of 10s intervals.
    
    // However, we can't display 20000 bars. We must aggregate.
    // Let's assume we want to display ~60-80 bars on screen.
    const displayBars = 60;
    
    let totalSeconds = 0;
    if (rangeVal === 'retention') {
        totalSeconds = retentionDays * 24 * 3600;
    } else {
        totalSeconds = parseInt(rangeVal) * 10;
    }

    // Each bar represents 'secondsPerBar'
    const secondsPerBar = Math.max(10, Math.ceil(totalSeconds / displayBars));

    data.forEach(item => {
        const card = document.createElement('div');
        card.className = 'monitor-card';
        
        const isUp = item.current_state;
        const statusClass = isUp ? 'up' : 'down';
        const statusText = isUp ? 'Online' : 'Offline';
        const statusColor = isUp ? 'text-success' : 'text-danger';
        
        // --- Aggregation Logic ---
        // item.records[0] is newest.
        // We want to slice the records based on totalSeconds.
        const recordsNeeded = Math.ceil(totalSeconds / 10);
        const records = item.records.slice(0, recordsNeeded).reverse(); // Oldest first
        
        // Group records into bars
        const barsData = [];
        // If records are fewer than displayBars, we might just show them directly if interval matches, 
        // but for consistency let's use time-based grouping.
        // Actually, simplest way: divide time range into 'displayBars' slots.
        // Start time = Now - totalSeconds. End time = Now.
        
        const now = Date.now();
        const startTime = now - (totalSeconds * 1000);
        const stepMs = (totalSeconds * 1000) / displayBars;
        
        let currentRecordIdx = 0;
        
        for (let i = 0; i < displayBars; i++) {
            const bucketStart = startTime + (i * stepMs);
            const bucketEnd = bucketStart + stepMs;
            
            // Find records in this bucket
            let bucketRecords = [];
            // records are sorted oldest first.
            while (currentRecordIdx < records.length) {
                const rTime = new Date(records[currentRecordIdx].timestamp).getTime();
                if (rTime < bucketStart) {
                    currentRecordIdx++; // Skip too old (shouldn't happen if sliced correctly)
                    continue;
                }
                if (rTime >= bucketEnd) {
                    break; // Belongs to next bucket
                }
                bucketRecords.push(records[currentRecordIdx]);
                currentRecordIdx++;
            }
            
            if (bucketRecords.length === 0) {
                // If no record, checks if it's in future or just missing data
                if (bucketEnd > now) {
                    // Future - skip or empty?
                    // barsData.push({ type: 'empty' }); 
                } else {
                     // Missing data (or maybe service was down/stopped)
                     barsData.push({ type: 'empty' });
                }
            } else {
                // Aggregate
                const successCount = bucketRecords.filter(r => r.success).length;
                const failCount = bucketRecords.length - successCount;
                const avgLatency = bucketRecords.reduce((acc, r) => acc + (r.latency_ms || 0), 0) / bucketRecords.length;
                        
                        let type = 'ok';
                        if (failCount === bucketRecords.length) type = 'fail';
                        else if (failCount > 0) type = 'warning';
                        
                        barsData.push({
                            type: type,
                            time: new Date(bucketStart).toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'}),
                            latency: avgLatency < 1 && avgLatency > 0 ? avgLatency.toFixed(2) : Math.round(avgLatency),
                            count: bucketRecords.length,
                            fails: failCount
                        });
            }
        }
        
        // Uptime calc (based on full visible range)
        const totalRecs = records.length;
        const totalSuccess = records.filter(r => r.success).length;
        const uptime = totalRecs > 0 ? ((totalSuccess / totalRecs) * 100).toFixed(1) : '0.0';
        
        const protocol = item.target.protocol;
        let targetStr = '';
        if (protocol === 'ICMP') {
            targetStr = item.target.host;
        } else if (protocol === 'HTTP' || protocol === 'HTTPS') {
            // For Web, showing the host is usually enough, or host:port if non-standard
            targetStr = item.target.host;
            if (item.target.port) {
                targetStr += `:${item.target.port}`;
            }
        } else {
            targetStr = `${item.target.host}:${item.target.port || '?'}`;
        }
        
        const barsHtml = barsData.map(b => {
            if (b.type === 'empty') return `<div class="bar-segment empty"></div>`;
            
            const title = `${b.time}\nAvg Latency: ${b.latency}ms\nSuccess: ${b.count - b.fails}/${b.count}`;
            return `<div class="bar-segment ${b.type}" data-title="${title}"></div>`;
        }).join('');

        card.innerHTML = `
            <div class="m-header">
                <div class="m-info">
                    <span class="m-name">${item.target.name}</span>
                    <span class="m-target">${targetStr}</span>
                    <span class="m-meta">| ${protocol} | Uptime: ${uptime}% (${rangeVal === 'retention' ? retentionDays + 'd' : Math.round(totalSeconds/3600) + 'h'})</span>
                </div>
                <div class="m-status ${statusColor}">
                    <span class="status-dot ${statusClass}"></span> ${statusText}
                </div>
            </div>
            <div class="status-bar">
                ${barsHtml}
            </div>
        `;
        list.appendChild(card);
    });
    
    renderGlobalEventLog(data);
}

function renderGlobalEventLog(data) {
    const allEvents = [];
    
    data.forEach(item => {
        if (item.records.length > 1) {
            for (let i = 0; i < item.records.length - 1; i++) {
                const curr = item.records[i];
                const prev = item.records[i+1];
                if (curr.success !== prev.success) {
                    allEvents.push({
                        target: item.target.name,
                        type: curr.success ? 'UP' : 'DOWN',
                        time: new Date(curr.timestamp), // Keep as Date object for sorting
                        msg: curr.message || (curr.success ? 'Recovered' : 'Unknown Error')
                    });
                }
            }
        }
    });
    
    // Sort by time descending
    allEvents.sort((a, b) => b.time - a.time);
    
    // Limit to 20
    const recentEvents = allEvents.slice(0, 20);
    
    const tbody = document.getElementById('global-event-body');
    tbody.innerHTML = '';
    
    if (recentEvents.length === 0) {
        tbody.innerHTML = '<tr><td colspan="4" style="text-align:center; color:var(--text-muted); padding:20px;">No status changes recorded in current history.</td></tr>';
        return;
    }
    
    recentEvents.forEach(e => {
        const row = document.createElement('tr');
        row.innerHTML = `
            <td class="event-time">${e.time.toLocaleString()}</td>
            <td><span class="event-badge ${e.type.toLowerCase()}">${e.type}</span></td>
            <td class="event-target">${e.target}</td>
            <td class="event-msg">${e.msg}</td>
        `;
        tbody.appendChild(row);
    });
}

function startDashboardUpdates() {
    if (eventSource) {
        eventSource.close();
    }

    eventSource = new EventSource('/api/events');

    eventSource.addEventListener('init', (e) => {
        try {
            monitorData = JSON.parse(e.data);
            renderDashboard(monitorData);
            document.getElementById('last-updated').innerText = 'Connected via SSE';
        } catch (err) {
            console.error('Failed to parse init data', err);
        }
    });

    eventSource.addEventListener('update', (e) => {
        try {
            const updatedStatus = JSON.parse(e.data);
            const index = monitorData.findIndex(item => item.target.id === updatedStatus.target.id);
            if (index !== -1) {
                monitorData[index] = updatedStatus;
            } else {
                // New target? or reordered? Just push it for now or reload
                monitorData.push(updatedStatus);
            }
            // Re-render
            renderDashboard(monitorData);
            // document.getElementById('last-updated').innerText = 'Last updated: ' + new Date().toLocaleTimeString();
        } catch (err) {
            console.error('Failed to parse update data', err);
        }
    });

    eventSource.onerror = (err) => {
        console.error('SSE Error', err);
        document.getElementById('last-updated').innerText = 'Connection lost, reconnecting...';
        // EventSource automatically reconnects, but we might want to handle visual state
    };
}

function stopDashboardUpdates() {
    if (eventSource) {
        eventSource.close();
        eventSource = null;
    }
}

// --- Config Logic ---
async function loadConfig(renderForm = true) {
    try {
        const res = await fetch('/api/config');
        currentConfig = await res.json();
        if (renderForm) renderConfigForm();
    } catch (e) {
        alert('Failed to load config: ' + e.message);
    }
}

function renderConfigForm() {
    if (!currentConfig) return;
    const tbody = document.getElementById('config-targets-body');
    tbody.innerHTML = '';
    (currentConfig.targets || []).forEach(t => addConfigRow(t));

    // Webhook Logic
    const webhookBody = document.getElementById('config-webhooks-body');
    webhookBody.innerHTML = '';
    (currentConfig.alert?.webhooks || []).forEach(w => addWebhookRow(w));

    document.getElementById('config-retention').value = currentConfig.data_retention_days || 3;
}

function addTargetRow() {
    addConfigRow({
        id: 't_' + Date.now(),
        name: 'SSH',
        host: 'localhost',
        port: 22,
        protocol: 'TCP',
        threshold: 3
    });
}

function addConfigRow(target) {
    const tbody = document.getElementById('config-targets-body');
    const row = document.createElement('tr');
    row.className = 'target-row';
    
    const protoOptions = ['TCP', 'ICMP', 'DNS', 'HTTP', 'HTTPS'].map(p => 
        `<option value="${p}" ${target.protocol === p ? 'selected' : ''}>${p}</option>`
    ).join('');

    row.innerHTML = `
        <input type="hidden" class="c-id" value="${target.id}">
        <td><input type="text" class="c-name" value="${target.name}"></td>
        <td>
            <select class="c-proto" onchange="updateRowState(this.closest('tr'), true)">
                ${protoOptions}
            </select>
        </td>
        <td><input type="text" class="c-host" value="${target.host}"></td>
        <td><input type="number" class="c-port" value="${target.port !== null ? target.port : ''}" placeholder="N/A"></td>
        <td><input type="number" class="c-threshold" value="${target.threshold || 3}" min="1" max="20" style="width: 60px;"></td>
        <td><button class="btn btn-danger btn-sm" onclick="this.closest('tr').remove()">Delete</button></td>
    `;
    tbody.appendChild(row);
    updateRowState(row);
}

function updateRowState(row, fromUserChange = false) {
    const proto = row.querySelector('.c-proto').value;
    const portInput = row.querySelector('.c-port');

    if (proto === 'ICMP') {
        portInput.disabled = true;
        portInput.value = '';
        portInput.placeholder = 'N/A';
    } else {
        portInput.disabled = false;
        const defPort = DEFAULT_PORTS[proto];
        portInput.placeholder = defPort;
        if (fromUserChange) {
            portInput.value = defPort;
        }
    }
}

// --- Webhook Logic ---
function addWebhookRow(webhook) {
    const tbody = document.getElementById('config-webhooks-body');
    const row = document.createElement('tr');
    row.className = 'webhook-row';
    
    // Default values
    const id = webhook?.id || 'w_' + Date.now();
    
    // Defaults for new rows (Telegram)
    const defName = 'Telegram';
    const defUrl = "https://api.telegram.org/bot<your_bot_token>/sendMessage";
    const defTmpl = '{"chat_id":"<your_chat_id>","text":"{{STATUS}} {{TARGET}} {{HOST}} {{TIME}} {{MESSAGE}}"}';

    const name = webhook ? (webhook.name || '') : defName;
    const url = webhook ? (webhook.url || '') : defUrl;
    const tmpl = webhook ? (webhook.template || '') : defTmpl;
    const enabled = webhook?.enabled !== undefined ? webhook.enabled : true;

    row.innerHTML = `
        <input type="hidden" class="w-id" value="${id}">
        <td><input type="checkbox" class="w-enabled" ${enabled ? 'checked' : ''}></td>
        <td><input type="text" class="w-name" value="${name}" placeholder="Name"></td>
        <td>
            <div style="display:flex; flex-direction:column; gap:4px;">
                <input type="text" class="w-url" value="${url}" placeholder="https://..." style="width: 100%; box-sizing: border-box;">
                <textarea class="w-template" placeholder='Optional Template JSON...' style="height: 40px; font-family:monospace; font-size:0.8rem; width: 100%; box-sizing: border-box; resize: none; white-space: nowrap; overflow: hidden;">${tmpl}</textarea>
            </div>
        </td>
        <td style="vertical-align:top;"><button class="btn btn-danger btn-sm" onclick="this.closest('tr').remove()">Delete</button></td>
    `;
    tbody.appendChild(row);
}

async function saveConfig() {
    // Collect Targets
    const targets = [];
    document.querySelectorAll('#config-targets-body tr').forEach(row => {
        const proto = row.querySelector('.c-proto').value;
        let portVal = row.querySelector('.c-port').value;
        let port = parseInt(portVal);

        if (proto === 'ICMP') { 
            port = null; 
        } else if (isNaN(port)) {
            if (DEFAULT_PORTS[proto]) port = DEFAULT_PORTS[proto];
        }

        targets.push({
            id: row.querySelector('.c-id').value,
            name: row.querySelector('.c-name').value,
            host: row.querySelector('.c-host').value,
            port: isNaN(port) ? null : port,
            protocol: proto,
            threshold: parseInt(row.querySelector('.c-threshold').value) || 3
        });
    });

    // Collect Webhooks
    const webhooks = [];
    document.querySelectorAll('#config-webhooks-body tr').forEach(row => {
        const tmplVal = row.querySelector('.w-template').value.trim();
        webhooks.push({
            id: row.querySelector('.w-id').value,
            name: row.querySelector('.w-name').value,
            url: row.querySelector('.w-url').value,
            template: tmplVal ? tmplVal : null,
            enabled: row.querySelector('.w-enabled').checked
        });
    });

    const newConfig = {
        targets: targets,
        alert: {
            enabled: true,
            webhooks: webhooks
        },
        data_retention_days: parseInt(document.getElementById('config-retention').value) || 3
    };

    try {
        const res = await fetch('/api/config', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(newConfig)
        });
        const result = await res.json();
        
        if (result.success) {
            alert('Configuration saved successfully!');
            currentConfig = newConfig;
            retentionDays = newConfig.data_retention_days;
            // Update dropdown
            const retentionOpt = document.querySelector('option[value="retention"]');
            if (retentionOpt) retentionOpt.text = `Retention Policy (${retentionDays} Days)`;
            
            toggleSettings(); // Switch back to dashboard
        } else {
            alert('Error saving config: ' + (result.error || JSON.stringify(result)));
        }
    } catch (e) {
        alert('Network error: ' + e.message);
    }
}

// Init
init();
