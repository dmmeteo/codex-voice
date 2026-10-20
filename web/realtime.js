const MAX_EVENT_BYTES = 64 * 1024;

export function normalizeEvent(raw) {
  if (typeof raw !== 'string' || raw.length > MAX_EVENT_BYTES) return null;
  try {
    let event = JSON.parse(raw);
    if (event?.type === 'data_message') {
      event = typeof event.data === 'string' ? JSON.parse(event.data) : event.data;
    }
    return event && typeof event === 'object' && !Array.isArray(event) && typeof event.type === 'string' ? event : null;
  } catch { return null; }
}

export class RealtimeSession {
  constructor({audio, onState, onEvent, fetch = (...args) => globalThis.fetch(...args)}) {
    this.audio = audio;
    this.fetch = fetch;
    this.onState = onState;
    this.onEvent = onEvent;
    this.phase = 'idle';
    this.generation = 0;
    this.peer = null;
    this.stream = null;
    this.channel = null;
    this.abort = null;
    this.timer = null;
    this.recoveryTimer = null;
    this.readiness = null;
    this.sessionId = null;
  }

  setState(phase, detail = '') {
    this.phase = phase;
    this.onState({phase, detail, sessionId: this.sessionId});
  }

  async start(options, apiKey = '') {
    if (this.phase !== 'idle') return;
    this.cleanup();
    const generation = this.generation;
    const current = () => generation === this.generation;
    this.readiness = {transportConnected: false, channelOpen: false, backendReady: options.mode === 'standard'};
    this.setState('starting', 'Requesting microphone access…');
    this.abort = new AbortController();
    const signal = this.abort.signal;
    this.timer = setTimeout(() => { if (current()) this.fail('Connection timed out. Try again.'); }, 90_000);
    try {
      if (!navigator.mediaDevices?.getUserMedia || !globalThis.RTCPeerConnection) {
        throw new Error('Voice needs a browser with WebRTC on localhost or HTTPS.');
      }
      const stream = await navigator.mediaDevices.getUserMedia({audio: {echoCancellation: true, noiseSuppression: true}, video: false});
      if (!current()) { stream.getTracks().forEach(track => track.stop()); return; }
      this.stream = stream;
      const peer = new RTCPeerConnection({bundlePolicy: 'max-bundle'});
      this.peer = peer;
      peer.addTransceiver(stream.getAudioTracks()[0], {direction: 'sendrecv', streams: [stream]});
      const channel = peer.createDataChannel('', {negotiated: true, id: 0});
      this.channel = channel;
      channel.onmessage = ({data}) => {
        if (!current()) return;
        const event = normalizeEvent(data);
        if (!event) { this.onEvent({type: 'unrecognized_event', payload: {message: 'Ignored malformed or oversized data.'}}); return; }
        this.onEvent(event);
        if (event.type === 'startup_telemetry' && event.payload?.success === true) {
          this.readiness.backendReady = true;
          this.#updateReadiness();
        }
        if (event.type === 'goodbye') this.stop('The voice service ended the session.');
        if (event.type === 'startup_telemetry' && event.payload?.success === false) {
          this.fail(options.mode === 'advanced'
            ? 'ChatGPT could not start Advanced voice. Try Standard mode.'
            : 'ChatGPT could not start the voice conversation. Try again later.');
        }
        if (event.type === 'error') {
          const message = typeof event.error?.message === 'string'
            ? event.error.message.replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ').trim().slice(0, 240)
            : '';
          this.fail(message || 'The voice service reported an error. See the event log.');
        }
      };
      channel.onopen = () => {
        if (!current() || channel.readyState !== 'open') return;
        this.readiness.channelOpen = true;
        this.onEvent({type: 'data_channel_open'});
        if (this.#sendMicrophoneState(generation)) this.#updateReadiness();
      };
      channel.onclose = () => { if (current()) this.fail('The voice data channel closed. Start a new session.'); };
      channel.onerror = () => { if (current()) this.fail('The voice data channel failed. Start a new session.'); };
      peer.ontrack = ({track, streams}) => {
        if (!current()) { track.stop(); return; }
        this.audio.srcObject = streams[0] || new MediaStream([track]);
        if (this.phase !== 'paused') this.play(generation);
      };
      peer.onconnectionstatechange = () => {
        if (!current()) return;
        this.onEvent({type: 'connection_state', payload: {state: peer.connectionState}});
        this.readiness.transportConnected = peer.connectionState === 'connected';
        if (peer.connectionState === 'connected') {
          clearTimeout(this.recoveryTimer);
          this.recoveryTimer = null;
          this.#updateReadiness();
        } else if (['failed', 'closed'].includes(peer.connectionState)) {
          this.fail('The voice connection ended. Start a new session.');
        } else if (peer.connectionState === 'disconnected') {
          this.setState(this.phase, 'Connection interrupted. Waiting to reconnect…');
          clearTimeout(this.recoveryTimer);
          this.recoveryTimer = setTimeout(() => { if (current()) this.fail('The voice connection could not recover.'); }, 15_000);
        }
      };
      await peer.setLocalDescription(await peer.createOffer());
      if (!current()) return;
      await gatherIce(peer, signal);
      if (!current()) return;
      this.setState('starting', 'Connecting to ChatGPT…');
      const headers = {'Content-Type': 'application/json'};
      if (apiKey.trim()) headers.Authorization = `Bearer ${apiKey.trim()}`;
      const response = await this.fetch('/v1/realtime/sessions', {method: 'POST', headers, signal, body: JSON.stringify({...options, sdp: peer.localDescription.sdp})});
      if (!current()) return;
      const result = await response.json().catch(() => null);
      if (!current()) return;
      if (response.status === 401) throw new Error("Authentication failed (401). Enter this server's API key above, or reload the page to sign in again.");
      if (!response.ok) throw new Error(result?.error?.message || `Realtime request failed (${response.status}).`);
      if (!result) throw new Error('The server returned an unreadable response.');
      if (typeof result.sdp !== 'string' || result.sdp.length > 128 * 1024 || typeof result.session_id !== 'string') throw new Error('The server returned an invalid voice answer.');
      this.sessionId = result.session_id;
      await peer.setRemoteDescription({type: 'answer', sdp: result.sdp});
      if (!current()) return;
      this.setState(this.phase, this.phase === 'active' ? 'Connected. Speak naturally.' : 'Waiting for voice service…');
    } catch (error) {
      if (current()) this.fail(error?.name === 'NotAllowedError' ? 'Microphone access was denied. Allow access and try again.' : error.message || 'Could not start voice.');
    }
  }

  #updateReadiness() {
    if (this.phase !== 'starting') return;
    const {transportConnected, channelOpen, backendReady} = this.readiness;
    if (!transportConnected || !channelOpen || !backendReady) {
      this.setState('starting', 'Waiting for voice service…');
      return;
    }
    clearTimeout(this.timer);
    this.timer = null;
    this.setState('active', 'Connected. Speak naturally.');
  }

  #sendMicrophoneState(generation = this.generation) {
    if (generation !== this.generation) return false;
    if (this.channel?.readyState !== 'open') {
      this.fail('The voice data channel closed. Start a new session.');
      return false;
    }
    const track = this.stream?.getAudioTracks()[0];
    if (!track) return false;
    try {
      this.channel.send(JSON.stringify({type: 'data_message', data: JSON.stringify({
        type: 'track_state', payload: {type: 'track_state', track_id: 'microphone', media_type: 'audio', media_source: 'microphone', state: track.enabled ? 'live' : 'muted'},
      })}));
      return true;
    } catch {
      this.fail('The voice controls could not be sent. Start a new session.');
      return false;
    }
  }

  play(generation = this.generation) {
    this.audio.play().catch(() => {
      if (generation === this.generation && this.phase !== 'paused') this.setState(this.phase, 'Audio playback is blocked. Pause, then Resume to enable it.');
    });
  }

  pause() {
    if (this.phase !== 'active') return;
    this.stream?.getAudioTracks().forEach(track => { track.enabled = false; });
    if (!this.#sendMicrophoneState()) return;
    this.audio.muted = true;
    this.audio.pause();
    this.setState('paused', 'Microphone and playback paused locally. Server events may continue.');
  }

  resume() {
    if (this.phase !== 'paused') return;
    this.stream?.getAudioTracks().forEach(track => { track.enabled = true; });
    if (!this.#sendMicrophoneState()) return;
    this.audio.muted = false;
    this.setState('active', 'Connected. Speak naturally.');
    this.play();
  }

  stop(detail = 'Stopped. Microphone released.') {
    this.cleanup();
    this.setState('idle', detail);
  }

  fail(message) {
    this.cleanup();
    this.setState('idle', message);
    this.onEvent({type: 'session_error', payload: {message}});
  }

  cleanup() {
    this.generation += 1;
    clearTimeout(this.timer);
    this.timer = null;
    clearTimeout(this.recoveryTimer);
    this.recoveryTimer = null;
    this.readiness = null;
    this.abort?.abort();
    this.abort = null;
    if (this.channel) {
      this.channel.onclose = this.channel.onerror = this.channel.onmessage = this.channel.onopen = null;
      this.channel.close();
    }
    if (this.peer) {
      this.peer.ontrack = this.peer.onconnectionstatechange = null;
      this.peer.close();
    }
    this.stream?.getTracks().forEach(track => track.stop());
    this.audio.pause();
    this.audio.srcObject?.getTracks().forEach(track => track.stop());
    this.audio.srcObject = null;
    this.audio.muted = false;
    this.peer = this.channel = this.stream = null;
    this.sessionId = null;
  }
}

function gatherIce(peer, signal) {
  if (peer.iceGatheringState === 'complete') return Promise.resolve();
  return new Promise((resolve, reject) => {
    const finish = (error) => {
      clearTimeout(timer);
      peer.removeEventListener('icegatheringstatechange', changed);
      signal.removeEventListener('abort', aborted);
      error ? reject(error) : resolve();
    };
    const changed = () => { if (peer.iceGatheringState === 'complete') finish(); };
    const aborted = () => finish(new DOMException('Stopped', 'AbortError'));
    const timer = setTimeout(() => finish(new Error('ICE gathering timed out. Check your network.')), 15_000);
    peer.addEventListener('icegatheringstatechange', changed);
    signal.addEventListener('abort', aborted, {once: true});
    if (signal.aborted) aborted(); else changed();
  });
}

export class TranscriptLog {
  constructor() { this.entries = new Map(); this.channels = new Map(); this.previous = {c: 0, p: '', o: 'add'}; this.resolvedModelSlug = null; }

  receive(event) {
    const payload = event.payload;
    if (!payload || typeof payload !== 'object') return;
    let messages = [];
    if (event.type === 'full_chat_message') messages = [payload.message];
    if (event.type === 'streaming_message_update') messages = [payload.request, payload.response?.message];
    if (event.type === 'chat_message_delta') {
      try {
        const delta = payload.delta;
        if (!delta || typeof delta !== 'object') return;
        const decoded = {c: delta.c ?? delta.channel ?? this.previous.c, p: delta.p ?? delta.path ?? this.previous.p, o: delta.o ?? delta.op ?? this.previous.o, v: Object.hasOwn(delta, 'v') ? delta.v : delta.value};
        if (!Number.isInteger(decoded.c) || decoded.c < 0 || decoded.c > 63) return;
        this.previous = decoded;
        const value = applyDelta(structuredClone(this.channels.get(decoded.c)), decoded);
        if (JSON.stringify(value)?.length > MAX_EVENT_BYTES) { this.channels.delete(decoded.c); return; }
        this.channels.set(decoded.c, value);
        messages = [value?.message];
      } catch { return; }
    }
    for (const message of messages) {
      if (!message?.id || !message.content) continue;
      const role = message.author?.role;
      if (!['user', 'assistant'].includes(role) || message.metadata?.is_visually_hidden_from_conversation === true || (message.channel != null && message.channel !== 'final')) continue;
      const slug = message.metadata?.resolved_model_slug;
      if (role === 'assistant' && typeof slug === 'string' && slug.length <= 64 && /^[a-zA-Z0-9._-]+$/.test(slug)) this.resolvedModelSlug = slug;
      const parts = message.content.parts;
      const text = Array.isArray(parts) ? parts.map(part => typeof part === 'string' ? part : part?.text || part?.transcript || '').join('') : message.content.text;
      this.put(message.id, role, text);
    }
    if (!messages.some(message => message?.id && message.content) && ['user_transcription_text', 'live_captioning_text', 'streaming_message_update'].includes(event.type)) {
      const text = payload.text ?? payload.transcript;
      const id = payload.message_id ?? payload.item_id ?? payload.turn_id;
      if (typeof id === 'string') this.put(id, event.type === 'user_transcription_text' ? 'user' : 'assistant', text);
    }
  }

  put(id, role, text) {
    if (typeof id !== 'string' || typeof text !== 'string') return;
    this.entries.set(id.slice(0, 256), {role: String(role).slice(0, 32), text: text.slice(0, 16_000)});
    while (this.entries.size > 100) this.entries.delete(this.entries.keys().next().value);
  }
}

function applyDelta(root, delta, depth = 0) {
  if (depth > 32) throw new Error('Delta nesting limit');
  const path = delta.p ?? delta.path ?? '';
  if (typeof path !== 'string' || path.length > 1024) throw new Error('Invalid delta path');
  const keys = path === '' ? [] : path.replace(/^\//, '').split('/').map(k => k.replace(/~1/g, '/').replace(/~0/g, '~'));
  if (keys.some(k => ['__proto__', 'constructor', 'prototype'].includes(k))) throw new Error('Invalid delta key');
  const box = {root};
  let parent = box;
  let key = 'root';
  for (const next of keys) {
    if (parent[key] === undefined) parent[key] = /^\d+$/.test(next) ? [] : {};
    parent = parent[key];
    key = next;
    if (Array.isArray(parent) && (!/^\d+$/.test(key) || Number(key) > 4096)) throw new Error('Invalid array index');
  }
  const value = Object.hasOwn(delta, 'v') ? delta.v : delta.value;
  switch (delta.o ?? delta.op) {
    case 'replace': parent[key] = value; break;
    case 'add': if (Array.isArray(parent)) parent.splice(Number(key), 0, value); else parent[key] = value; break;
    case 'remove': if (Array.isArray(parent)) parent.splice(Number(key), 1); else delete parent[key]; break;
    case 'append':
      if (typeof parent[key] === 'string') parent[key] += value;
      else if (Array.isArray(parent[key])) parent[key].push(...(Array.isArray(value) ? value : [value]));
      else if (parent[key] && typeof parent[key] === 'object' && value && typeof value === 'object') {
        for (const k of Object.keys(value)) {
          if (['__proto__', 'constructor', 'prototype'].includes(k)) throw new Error('Invalid delta key');
          parent[key][k] = value[k];
        }
      } else parent[key] = value;
      break;
    case 'truncate':
      if (!Number.isInteger(value) || value < 0 || value > MAX_EVENT_BYTES) throw new Error('Invalid truncate');
      if (typeof parent[key] === 'string') parent[key] = parent[key].slice(0, value);
      else if (Array.isArray(parent[key])) parent[key].length = Math.min(value, parent[key].length);
      break;
    case 'patch':
      if (!Array.isArray(value) || value.length > 256) throw new Error('Invalid patch');
      for (const child of value) parent[key] = applyDelta(parent[key], child, depth + 1);
      break;
    default: throw new Error('Unknown delta operation');
  }
  return box.root;
}

const MAX_EVENT_ROWS = 150;

/** Binds the Realtime tab. `Session` is injectable for tests. */
export function mountRealtime(document, {apiKey, Session = RealtimeSession, now = () => new Date()} = {}) {
  const $ = id => document.getElementById(id);
  let transcript = new TranscriptLog();
  const transcriptRows = new Map();
  const session = new Session({
    audio: $('remote-audio'),
    onState: ({phase, detail, sessionId}) => {
      $('phase').textContent = phase[0].toUpperCase() + phase.slice(1);
      $('status').textContent = detail;
      $('session-id').textContent = sessionId ? `Session ${sessionId}` : '';
      renderStart();
      $('pause').disabled = !['active', 'paused'].includes(phase);
      $('pause').textContent = phase === 'paused' ? 'Resume' : 'Pause';
      $('stop').disabled = phase === 'idle';
      for (const id of ['mode', 'voice', 'transcripts-enabled', 'save-conversation']) $(id).disabled = phase !== 'idle';
    },
    onEvent: event => {
      const row = document.createElement('li');
      const heading = document.createElement('div');
      const body = document.createElement('pre');
      heading.textContent = `${now().toLocaleTimeString()}  ${event.type}`;
      body.textContent = JSON.stringify(event.payload ?? event, null, 2).slice(0, 4000);
      row.append(heading, body);
      $('events').prepend(row);
      while ($('events').children.length > MAX_EVENT_ROWS) $('events').lastChild.remove();
      transcript.receive(event);
      renderTranscript();
      renderModel();
    },
  });

  function canStart() {
    return session.phase === 'idle' && ($('mode').value === 'standard' || $('save-conversation').checked);
  }

  const READY = 'Ready. Start to allow microphone access.';
  const historyRequired = () => `${$('mode').value[0].toUpperCase()}${$('mode').value.slice(1)} voice requires Save conversation. Turn it on, or choose Standard.`;

  function renderStart() {
    $('start').disabled = !canStart();
    if (session.phase === 'idle' && !canStart()) $('status').textContent = historyRequired();
    else if (session.phase === 'idle' && $('status').textContent.endsWith('requires Save conversation. Turn it on, or choose Standard.')) $('status').textContent = READY;
    $('save-help').textContent = $('save-conversation').checked
      ? 'This conversation will be saved to ChatGPT history.'
      : $('mode').value === 'standard'
        ? 'Off keeps this conversation out of ChatGPT history.'
        : 'This mode requires saving to ChatGPT history. Turn on Save conversation, or choose Standard.';
  }

  // Rows are edited in place and new ones prepended so streaming never rebuilds the list under the reader.
  function renderTranscript() {
    for (const [id, row] of transcriptRows) {
      if (!transcript.entries.has(id)) { row.remove(); transcriptRows.delete(id); }
    }
    for (const [id, {role, text}] of transcript.entries) {
      let row = transcriptRows.get(id);
      if (!row) {
        row = document.createElement('article');
        row.append(document.createElement('strong'), document.createElement('p'));
        transcriptRows.set(id, row);
        $('transcripts').prepend(row);
      }
      const [label, body] = row.children;
      const speaker = role === 'user' ? 'You' : role === 'assistant' ? 'ChatGPT' : role;
      row.dataset.role = role;
      if (label.textContent !== speaker) label.textContent = speaker;
      if (body.textContent !== text) body.textContent = text;
    }
    $('transcript-empty').hidden = transcriptRows.size > 0;
  }

  function renderModel() {
    $('reported-model').textContent = $('transcripts-enabled').checked
      ? transcript.resolvedModelSlug || 'not reported yet'
      : 'not reported with transcripts off';
  }

  $('start').addEventListener('click', () => {
    if (!canStart()) return;
    transcript = new TranscriptLog();
    renderTranscript();
    renderModel();
    session.start({
      mode: $('mode').value,
      voice: $('voice').value,
      enable_message_streaming: $('transcripts-enabled').checked,
      history_and_training_disabled: !$('save-conversation').checked,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
      timezone_offset_minutes: now().getTimezoneOffset(),
    }, apiKey());
  });
  $('mode').addEventListener('change', renderStart);
  $('save-conversation').addEventListener('change', renderStart);
  $('transcripts-enabled').addEventListener('change', renderModel);
  $('pause').addEventListener('click', () => session.phase === 'paused' ? session.resume() : session.pause());
  $('stop').addEventListener('click', () => session.stop());
  $('clear').addEventListener('click', () => $('events').replaceChildren());

  renderModel();
  session.setState('idle', READY);
  return session;
}
