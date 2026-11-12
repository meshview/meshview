/* Read-only chat presentation using Meshview's existing packet API. */
document.addEventListener('DOMContentLoaded', async () => {
    const log = document.getElementById('chat-log');
    const channel = document.getElementById('channel-filter');
    const search = document.getElementById('chat-search');
    const status = document.getElementById('chat-status');
    const pause = document.getElementById('chat-pause');
    const latest = document.getElementById('chat-latest');
    // A channel in the filter, in its place among the others, unless it is
    // there already: those of the last day, and any other as its first message
    // arrives, so that a new channel shows without a reload.
    function addChannel(name) {
        if (!name) return;
        const options = [...channel.options].slice(1);
        if (options.some(option => option.value === name)) return;
        const after = options.find(option => option.value > name);
        channel.insertBefore(new Option(name, name), after || null);
    }
    const packets = new Map();
    let lastTime = null;
    let paused = false;
    let busy = false;
    let loaded = false;
    let unread = 0;
    // Older messages, a page at a time, from where the first page ended.
    let olderBefore = null, olderLoading = false, olderDone = false;
    let chatLang = {};
    const colors = ['#b54136', '#2563b4', '#23824d', '#8553af', '#a65d19', '#15817b', '#b63b79'];
    const timeOf = p => Number(p.import_time_us) || Date.parse(p.import_time) * 1000 || 0;
    const atBottom = () => log.scrollHeight - log.scrollTop - log.clientHeight < 60;
    const nodeName = p => (p.long_name || '').trim() || MeshI18n.t("Node {id}", {id: p.from_node_id});
    const packetUrl = id => `/packet/${encodeURIComponent(id)}`;
    function setStatus(text, state) { status.textContent = text; status.dataset.state = state; }
    // Unread messages show as a count badge on the "jump to latest" button.
    function showUnread() {
        const badge = document.getElementById('chat-unread');
        badge.textContent = unread;
        badge.hidden = !unread;
        latest.title = unread ? MeshI18n.t("{count} new messages", {count: unread}) : "";
        syncLatest();
    }
    // Nothing to jump to while the newest message is already in view.
    function syncLatest() { latest.disabled = atBottom(); }
    function goLatest() { log.scrollTop = log.scrollHeight; unread = 0; showUnread(); }

    function render(scrollToBottom = false, highlights = new Set()) {
        const query = search.value.trim().toLowerCase();
        const sorted = [...packets.values()].sort((a, b) => timeOf(a) - timeOf(b));
        const visible = sorted.filter(p => (!channel.value || (p.channel || '') === channel.value) &&
            `${nodeName(p)} ${p.payload || ''} ${p.from_node_id} ${p.channel || ''}`.toLowerCase().includes(query));
        const fragment = document.createDocumentFragment();
        if (packets.size) fragment.append(olderControl());
        let lastDay = '';
        const parents = new Map(sorted.map(p => [String(p.packet_id ?? p.id), p]));
        for (const packet of visible) {
            const date = new Date(timeOf(packet) / 1000);
            const day = date.toLocaleDateString(MeshI18n.language, { weekday:'long', month:'long', day:'numeric', year:'numeric' });
            if (day !== lastDay) {
                const separator = document.createElement('div');
                separator.className = 'chat-date-sep'; separator.textContent = day;
                fragment.append(separator); lastDay = day;
            }
            const row = document.createElement('article');
            row.className = `chat-line${highlights.has(String(packet.id)) ? ' flash' : ''}`;
            row.dataset.packetId = packet.id;
            let hash = 0;
            for (const char of String(packet.from_node_id)) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
            const time = document.createElement('time');
            time.className = 'chat-time'; time.dateTime = date.toISOString(); time.title = date.toLocaleString(MeshI18n.language);
            time.textContent = date.toLocaleTimeString(MeshI18n.language, {hour:'2-digit', minute:'2-digit', hour12:false});
            const nick = document.createElement('a');
            nick.className = 'chat-nick'; nick.href = `/node/${encodeURIComponent(packet.from_node_id)}`;
            nick.textContent = nodeName(packet); nick.title = nodeName(packet); nick.style.color = colors[hash % colors.length];
            const message = document.createElement('div'); message.className = 'chat-message';
            // Mesh payloads and node names are untrusted text, never markup.
            message.textContent = packet.payload ?? '';
            if (packet.reply_id) {
                const reply = document.createElement('span'); reply.className = 'replying-to';
                const parent = parents.get(String(packet.reply_id)) || packets.get(String(packet.reply_id));
                reply.append(`${chatLang.replying_to || MeshI18n.t("Replying to:")} `);
                if (parent) reply.append(`${nodeName(parent)}: ${parent.payload || ''}`);
                else {
                    const link = document.createElement('a'); link.href = packetUrl(packet.reply_id);
                    link.textContent = String(packet.reply_id); reply.append(link);
                }
                message.append(reply);
            }
            const meta = document.createElement('span'); meta.className = 'chat-meta';
            const tag = document.createElement('span'); tag.className = 'chat-channel';
            tag.textContent = packet.channel ? `# ${packet.channel}` : MeshI18n.t("Unknown channel"); tag.title = packet.channel || MeshI18n.t("Unknown channel");
            const details = document.createElement('a'); details.className = 'chat-details'; details.href = packetUrl(packet.id);
            details.textContent = MeshI18n.t("Details ↗"); details.title = chatLang.view_packet_details || MeshI18n.t("View packet details");
            details.setAttribute('aria-label', details.title);
            meta.append(tag, details); row.append(time, nick, message, meta); fragment.append(row);
        }
        if (!visible.length) {
            const empty = document.createElement('p'); empty.className = 'chat-empty';
            empty.textContent = packets.size ? MeshI18n.t("No messages match your filters.") : MeshI18n.t("No text messages received yet.");
            fragment.append(empty);
        }
        const previousTop = log.scrollTop;
        log.replaceChildren(fragment);
        document.getElementById('chat-count').textContent = MeshI18n.t(visible.length === 1 ? MeshI18n.t("{count} message") : MeshI18n.t("{count} messages"), {count: visible.length});
        if (scrollToBottom) goLatest(); else log.scrollTop = previousTop;
        syncLatest();
    }

    // The top of the list: earlier messages to load, loading, or no more.
    function olderControl() {
        const top = document.createElement('div');
        top.className = 'chat-older';
        if (olderDone) {
            top.textContent = MeshI18n.t("No earlier messages.");
        } else if (olderLoading) {
            top.textContent = MeshI18n.t("Loading earlier messages…");
        } else {
            const button = document.createElement('button');
            button.type = 'button'; button.className = 'chat-control';
            button.textContent = MeshI18n.t("Load earlier messages");
            button.addEventListener('click', loadOlder);
            top.append(button);
        }
        return top;
    }

    // The page before the oldest message loaded, keeping in view what was.
    async function loadOlder() {
        if (olderLoading || olderDone || !loaded) return;
        olderLoading = true;
        render();
        try {
            const url = new URL('/api/v1/packets', location.origin);
            url.searchParams.set('portnum', '1'); url.searchParams.set('limit', '100');
            url.searchParams.set('before', olderBefore);
            const response = await fetch(url);
            if (!response.ok) throw new Error(`HTTP ${response.status}`);
            const data = await response.json();
            if (!Array.isArray(data.packets)) throw new Error('Invalid packet response');
            for (const packet of data.packets) {
                if (!packets.has(String(packet.id))) packets.set(String(packet.id), packet);
                addChannel(packet.channel);
            }
            olderBefore = data.next_before ?? null;
            olderDone = olderBefore === null;
        } catch (error) {
            console.error('Chat older:', error);
        } finally {
            olderLoading = false;
            const height = log.scrollHeight, top = log.scrollTop;
            render();
            log.scrollTop = top + log.scrollHeight - height;
        }
    }

    async function fetchMessages() {
        if (paused || busy || document.hidden) return;
        busy = true;
        try {
            const url = new URL('/api/v1/packets', location.origin);
            url.searchParams.set('portnum', '1'); url.searchParams.set('limit', '100');
            if (lastTime) url.searchParams.set('since', lastTime);
            const response = await fetch(url);
            if (!response.ok) throw new Error(`HTTP ${response.status}`);
            const data = await response.json();
            if (!Array.isArray(data.packets)) throw new Error('Invalid packet response');
            const follow = !loaded || atBottom();
            const added = new Set();
            for (const packet of data.packets) {
                const id = String(packet.id);
                if (!packets.has(id)) { packets.set(id, packet); added.add(id); }
                addChannel(packet.channel);
            }
            // The first page says where the older ones start.
            if (!loaded) {
                olderBefore = data.next_before ?? null;
                olderDone = olderBefore === null;
            }
            if (!loaded || added.size) render(follow, loaded ? added : new Set());
            if (loaded && !follow && added.size) {
                unread += [...added].filter(id => {
                    const p = packets.get(id);
                    return p && (!channel.value || p.channel === channel.value) &&
                        `${nodeName(p)} ${p.payload || ''} ${p.from_node_id} ${p.channel || ''}`.toLowerCase().includes(search.value.trim().toLowerCase());
                }).length;
                showUnread();
            }
            lastTime = data.latest_import_time || lastTime;
            loaded = true;
            setStatus(paused ? MeshI18n.t("Paused") : MeshI18n.t("Live"), paused ? 'paused' : 'live');
            document.getElementById('chat-updated').textContent = MeshI18n.t("Updated {time}", {time: new Date().toLocaleTimeString(MeshI18n.language, {hour:"2-digit",minute:"2-digit"})});
        } catch (error) {
            console.error('Chat fetch:', error);
            if (!paused) setStatus(MeshI18n.t("Retrying…"), 'error');
            if (!loaded) log.innerHTML = `<p class="chat-empty">${MeshI18n.h("Unable to load messages. Retrying automatically…")}</p>`;
        } finally { busy = false; }
    }

    channel.addEventListener('change', () => render(true));
    search.addEventListener('input', () => render(true));
    latest.addEventListener('click', goLatest);
    log.addEventListener('scroll', () => {
        if (atBottom()) { unread = 0; showUnread(); } else syncLatest();
        // Only in a list long enough to scroll, and scrolled near its top: a
        // short, filtered one sits at the top without asking for the whole
        // history, and has the button for that.
        const scrolls = log.scrollHeight > log.clientHeight + 300;
        if (scrolls && log.scrollTop < 150) loadOlder();
    });
    pause.addEventListener('click', () => {
        paused = !paused; pause.setAttribute('aria-pressed', String(paused));
        pause.textContent = paused ? MeshI18n.t("Resume") : MeshI18n.t("Pause");
        setStatus(paused ? MeshI18n.t("Paused") : MeshI18n.t("Connecting…"), paused ? 'paused' : 'loading');
        if (!paused) fetchMessages();
    });
    document.addEventListener('visibilitychange', () => { if (!document.hidden) fetchMessages(); });

    // Load optional translations and channel metadata without delaying messages.
    (async () => {
        try {
            const cfg = await window._siteConfigPromise;
            const response = await fetch(`/api/v1/lang?lang=${encodeURIComponent(cfg?.site?.language || 'en')}&section=chat`);
            if (!response.ok) return;
            chatLang = await response.json();
            document.querySelectorAll('[data-chat-translate]').forEach(el => {
                if (chatLang[el.dataset.chatTranslate]) el.textContent = chatLang[el.dataset.chatTranslate];
            });
            if (loaded) render(atBottom());
        } catch (error) { console.error('Chat translations:', error); }
    })();
    (async () => {
        try {
            const response = await fetch('/api/v1/channels');
            if (!response.ok) return;
            const data = await response.json();
            for (const name of data.channels || []) addChannel(name);
        } catch (error) { console.error('Chat channels:', error); }
    })();
    await fetchMessages();
    setInterval(fetchMessages, 5000);
});
