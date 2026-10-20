import test from 'node:test';
import assert from 'node:assert/strict';
import {RealtimeSession, TranscriptLog, normalizeEvent} from '../web/realtime.js';

function deferred() { let resolve; const promise = new Promise(r => { resolve = r; }); return {promise, resolve}; }
function fixture() {
  const track = {enabled: true, stopped: false, stop() { this.stopped = true; }};
  const stream = {getTracks: () => [track], getAudioTracks: () => [track]};
  const audio = {srcObject: null, muted: false, playing: false, play() { this.playing = true; return Promise.resolve(); }, pause() { this.playing = false; }};
  const states = [], events = [];
  const session = new RealtimeSession({audio, onState: state => states.push(state), onEvent: event => events.push(event)});
  return {track, stream, audio, session, states, events};
}
function microphone(getUserMedia) { Object.defineProperty(globalThis, 'navigator', {configurable: true, value: {mediaDevices: {getUserMedia}}}); }
class Peer {
  static instances = [];
  constructor(configuration) { Peer.instances.push(this); this.configuration = configuration; this.iceGatheringState = 'complete'; this.channel = {readyState: 'connecting', sent: [], send(data) { this.sent.push(data); }, open() { this.readyState = 'open'; this.onopen?.(); }, close() { this.closed = true; this.readyState = 'closed'; }}; }
  addTransceiver(track, options) { assert.equal(options.direction, 'sendrecv'); this.track = track; }
  createDataChannel(label, options) { assert.equal(label, ''); assert.deepEqual(options, {negotiated: true, id: 0}); return this.channel; }
  async createOffer() { return {type: 'offer', sdp: 'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n'}; }
  async setLocalDescription(offer) { this.localDescription = offer; }
  async setRemoteDescription(answer) { this.remoteDescription = answer; this.connectionState = 'connected'; this.onconnectionstatechange?.(); this.channel.open(); }
  close() { this.closed = true; }
}
globalThis.RTCPeerConnection = Peer;

const answer = {ok: true, json: async () => ({sdp: 'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n', session_id: 'session-id'})};

test('Stop releases a microphone granted after cancellation', async () => {
  const f = fixture(), permission = deferred();
  microphone(() => permission.promise);
  const started = f.session.start({mode: 'standard'});
  assert.equal(f.session.phase, 'starting');
  f.session.stop();
  permission.resolve(f.stream);
  await started;
  assert.equal(f.track.stopped, true);
  assert.equal(f.session.phase, 'idle');
  assert.equal(f.session.peer, null);
});

test('Stop ignores a late answer and a new session can start', async () => {
  const f = fixture(), response = deferred(), requested = deferred();
  microphone(async () => f.stream);
  globalThis.fetch = () => { requested.resolve(); return response.promise; };
  const started = f.session.start({mode: 'standard'});
  await requested.promise;
  const peer = f.session.peer;
  f.session.stop();
  response.resolve(answer);
  await started;
  assert.equal(peer.remoteDescription, undefined);
  assert.equal(peer.closed, true);
  assert.equal(f.track.stopped, true);
  globalThis.fetch = async () => answer;
  await f.session.start({mode: 'standard'});
  assert.equal(f.session.phase, 'active');
  f.session.stop();
});

test('Pause and Resume control capture and playback; Stop releases both', async () => {
  const f = fixture();
  microphone(async () => f.stream);
  globalThis.fetch = async () => answer;
  await f.session.start({mode: 'standard'});
  f.session.pause();
  assert.equal(f.track.enabled, false);
  assert.equal(f.audio.muted, true);
  assert.equal(f.audio.playing, false);
  assert.equal(f.session.phase, 'paused');
  f.session.resume();
  assert.equal(f.track.enabled, true);
  assert.equal(f.audio.muted, false);
  assert.equal(f.audio.playing, true);
  f.session.stop();
  assert.equal(f.track.stopped, true);
  assert.equal(f.audio.srcObject, null);
});

for (const {name, event, detail} of [
  {
    name: 'Failed startup telemetry',
    event: {type: 'data_message', data: {type: 'startup_telemetry', payload: {success: false}}},
    detail: 'ChatGPT could not start the voice conversation. Try again later.',
  },
  {
    name: 'Outer service error',
    event: {type: 'error', error: {message: 'Voice session access denied.', code: 'forbidden'}},
    detail: 'Voice session access denied.',
  },
  {
    name: 'Service error with a non-string message',
    event: {type: 'error', error: {message: {code: 'forbidden'}}},
    detail: 'The voice service reported an error. See the event log.',
  },
  {
    name: 'Service error with an empty message',
    event: {type: 'error', error: {message: '\n\u0000\t'}},
    detail: 'The voice service reported an error. See the event log.',
  },
  {
    name: 'Service error with a long message and control characters',
    event: {type: 'error', error: {message: '\u0000Access\n' + 'x'.repeat(300)}},
    detail: 'Access ' + 'x'.repeat(233),
  },
]) {
  test(`${name} releases media and permits a new session`, async () => {
    const f = fixture(), remote = fixture(), retry = fixture();
    microphone(async () => f.stream);
    globalThis.fetch = async () => answer;
    await f.session.start({mode: 'standard'});
    const peer = f.session.peer, channel = f.session.channel;
    peer.ontrack({track: remote.track, streams: [remote.stream]});
    channel.onmessage({data: JSON.stringify(event)});
    assert.equal(f.session.phase, 'idle');
    assert.equal(f.states.at(-1).detail, detail);
    assert.equal(f.events.at(-1).payload.message, detail);
    assert.equal(f.track.stopped, true);
    assert.equal(remote.track.stopped, true);
    assert.equal(f.audio.srcObject, null);
    assert.equal(peer.closed, true);
    assert.equal(channel.closed, true);
    assert.equal(f.session.timer, null);
    microphone(async () => retry.stream);
    await f.session.start({mode: 'standard'});
    assert.equal(f.session.phase, 'active');
    assert.equal(retry.track.stopped, false);
    f.session.stop();
    assert.equal(retry.track.stopped, true);
  });
}

for (const mode of ['advanced', 'standard', 'wingman']) {
  test(`${mode} startup failure gives mode-specific recovery guidance`, async () => {
    const f = fixture();
    microphone(async () => f.stream);
    globalThis.fetch = async () => answer;
    await f.session.start({mode});
    f.session.channel.onmessage({data: JSON.stringify({type: 'startup_telemetry', payload: {success: false}})});
    assert.equal(f.session.phase, 'idle');
    assert.equal(f.track.stopped, true);
    assert.equal(f.states.at(-1).detail, mode === 'advanced'
      ? 'ChatGPT could not start Advanced voice. Try Standard mode.'
      : 'ChatGPT could not start the voice conversation. Try again later.');
  });
}

test('Startup telemetry only fails the session for an explicit false success value', async () => {
  const f = fixture();
  microphone(async () => f.stream);
  globalThis.fetch = async () => answer;
  await f.session.start({mode: 'standard'});
  for (const payload of [{success: true}, {}, {success: 'false'}]) {
    f.session.channel.onmessage({data: JSON.stringify({type: 'startup_telemetry', payload})});
    assert.equal(f.session.phase, 'active');
    assert.equal(f.track.stopped, false);
  }
  f.session.stop();
});

test('Private envelopes and transcript snapshots/deltas are normalized and bounded', () => {
  assert.equal(normalizeEvent('{bad'), null);
  assert.equal(normalizeEvent('x'.repeat(65537)), null);
  for (const data of [{type: 'future_event'}, JSON.stringify({type: 'future_event'})]) assert.equal(normalizeEvent(JSON.stringify({type: 'data_message', data})).type, 'future_event');
  const log = new TranscriptLog();
  log.receive({type: 'chat_message_delta', payload: {delta: {c: 0, p: '', o: 'add', v: {message: {id: 'm1', author: {role: 'assistant'}, content: {parts: ['Hello']}}}}}});
  log.receive({type: 'chat_message_delta', payload: {delta: {p: '/message/content/parts/0', o: 'append', v: ' world'}}});
  assert.equal(log.entries.get('m1').text, 'Hello world');
  log.receive({type: 'full_chat_message', payload: {message: {id: 'm1', author: {role: 'assistant'}, content: {parts: ['Final text']}}}});
  assert.equal(log.entries.size, 1);
  assert.equal(log.entries.get('m1').text, 'Final text');
  log.receive({type: 'chat_message_delta', payload: {delta: {p: '/__proto__/polluted', o: 'add', v: true}}});
  assert.equal({}.polluted, undefined);
  for (let i = 0; i < 105; i++) log.receive({type: 'user_transcription_text', payload: {message_id: `m${i}`, text: 'x'.repeat(20000)}});
  assert.equal(log.entries.size, 100);
  assert.equal(log.entries.get('m104').text.length, 16000);
});

test('Full snapshots display messages without overwriting delta channel state', () => {
  const log = new TranscriptLog();
  log.receive({type: 'chat_message_delta', payload: {delta: {c: 2, p: '', o: 'add', v: {message: {id: 'streamed', author: {role: 'assistant'}, content: {parts: ['Hello']}}}}}});
  log.receive({type: 'full_chat_message', payload: {message: {id: 'snapshot', author: {role: 'user'}, content: {parts: ['Question']}}}});
  log.receive({type: 'chat_message_delta', payload: {delta: {p: '/message/content/parts/0', o: 'append', v: ' world'}}});
  assert.deepEqual(log.entries.get('streamed'), {role: 'assistant', text: 'Hello world'});
  assert.deepEqual(log.entries.get('snapshot'), {role: 'user', text: 'Question'});
  log.receive({type: 'chat_message_delta', payload: {delta: {c: 3, p: '', o: 'add', v: {message: {id: 'snapshot', author: {role: 'user'}, content: {parts: ['Question']}}}}}});
  log.receive({type: 'chat_message_delta', payload: {delta: {p: '/message/content/parts/0', o: 'append', v: '?'}}});
  assert.deepEqual(log.entries.get('snapshot'), {role: 'user', text: 'Question?'});
  assert.deepEqual(log.entries.get('streamed'), {role: 'assistant', text: 'Hello world'});
});

test('Standard streaming requests and response echoes update the same transcript entry', () => {
  const log = new TranscriptLog();
  const request = {id: 'user-turn', author: {role: 'user'}, content: {content_type: 'text', parts: ['Please reply with only the word yes.']}, channel: null};
  log.receive({type: 'streaming_message_update', payload: {request, response: null}});
  assert.deepEqual([...log.entries.values()], [{role: 'user', text: 'Please reply with only the word yes.'}]);
  log.receive({type: 'streaming_message_update', payload: {request: null, response: {message: request, error: null}}});
  assert.equal(log.entries.size, 1);
  assert.deepEqual(log.entries.get('user-turn'), {role: 'user', text: 'Please reply with only the word yes.'});
});

test('Standard assistant snapshots replace text by message ID as the reply grows', () => {
  const log = new TranscriptLog();
  for (const text of ['', 'yes', 'yes']) {
    const message = {id: 'assistant-turn', author: {role: 'assistant'}, content: {content_type: 'text', parts: [text]}, channel: 'final'};
    log.receive({type: 'streaming_message_update', payload: {request: null, response: {message, error: null}}});
    assert.deepEqual([...log.entries.values()], [{role: 'assistant', text}]);
  }
});

test('Transcript snapshots and deltas exclude hidden, nonfinal, and nonconversation messages', () => {
  const log = new TranscriptLog();
  const excluded = [
    {author: {role: 'assistant'}, metadata: {is_visually_hidden_from_conversation: true}},
    {author: {role: 'user'}, metadata: {is_visually_hidden_from_conversation: true}},
    {author: {role: 'assistant'}, channel: 'analysis'},
    {author: {role: 'assistant'}, channel: 'commentary'},
    {author: {role: 'system'}},
    {author: {role: 'developer'}},
    {author: {role: 'tool'}},
  ];
  for (const [index, fields] of excluded.entries()) {
    const message = {id: `excluded-${index}`, content: {parts: ['Hidden content']}, channel: null, ...fields};
    for (const event of [
      {type: 'streaming_message_update', payload: {request: message, response: null}},
      {type: 'streaming_message_update', payload: {request: null, response: {message}}},
      {type: 'full_chat_message', payload: {message}},
      {type: 'chat_message_delta', payload: {delta: {c: 0, p: '', o: 'add', v: {message}}}},
    ]) {
      log.receive(event);
      assert.equal(log.entries.size, 0, `${event.type} exposed ${JSON.stringify(fields)}`);
    }
  }
});

test('Only bounded resolved model slugs from visible assistant messages are reported', () => {
  const log = new TranscriptLog();
  const message = (id, role, slug, fields = {}) => ({id, author: {role}, content: {parts: ['text']}, metadata: {resolved_model_slug: slug}, ...fields});
  log.receive({type: 'full_chat_message', payload: {message: message('user', 'user', 'user-model')}});
  log.receive({type: 'full_chat_message', payload: {message: message('hidden', 'assistant', 'hidden-model', {metadata: {resolved_model_slug: 'hidden-model', is_visually_hidden_from_conversation: true}})}});
  log.receive({type: 'full_chat_message', payload: {message: message('analysis', 'assistant', 'analysis-model', {channel: 'analysis'})}});
  assert.equal(log.resolvedModelSlug, null);
  log.receive({type: 'streaming_message_update', payload: {response: {message: message('reply', 'assistant', 'gpt6')}}});
  assert.equal(log.resolvedModelSlug, 'gpt6');
  log.receive({type: 'chat_message_delta', payload: {delta: {c: 0, p: '', o: 'add', v: {message: message('delta', 'assistant', 'gpt6astrawm')}}}});
  assert.equal(log.resolvedModelSlug, 'gpt6astrawm');
  log.receive({type: 'chat_message_delta', payload: {delta: {p: '/message/metadata/resolved_model_slug', o: 'replace', v: 'gpt6'}}});
  assert.equal(log.resolvedModelSlug, 'gpt6');
  for (const slug of ['x'.repeat(65), '<script>', '', 123]) {
    log.receive({type: 'full_chat_message', payload: {message: message(`bad-${slug}`, 'assistant', slug)}});
    assert.equal(log.resolvedModelSlug, 'gpt6');
  }
});

for (const mode of ['standard', 'advanced', 'wingman']) {
  for (const backendFirst of [true, false]) {
    test(`${mode} waits for transport, channel, and required backend readiness (${backendFirst})`, async () => {
      const f = fixture();
      microphone(async () => f.stream);
      globalThis.fetch = async () => answer;
      const original = Peer.prototype.setRemoteDescription;
      Peer.prototype.setRemoteDescription = async function(answer) { this.remoteDescription = answer; };
      try {
        await f.session.start({mode});
        const peer = f.session.peer, channel = f.session.channel;
        assert.deepEqual(peer.configuration, {bundlePolicy: 'max-bundle'});
        peer.connectionState = 'connected';
        peer.onconnectionstatechange();
        assert.equal(f.session.phase, 'starting');
        assert.notEqual(f.session.timer, null);
        const startup = () => channel.onmessage({data: JSON.stringify({type: 'startup_telemetry', payload: {success: true}})});
        if (backendFirst && mode !== 'standard') startup();
        assert.equal(f.session.phase, 'starting');
        channel.open();
        if (!backendFirst && mode !== 'standard') {
          assert.equal(f.session.phase, 'starting');
          assert.notEqual(f.session.timer, null);
          startup();
        }
        assert.equal(f.session.phase, 'active');
        assert.equal(f.session.timer, null);
        f.session.pause();
        startup();
        peer.onconnectionstatechange();
        assert.equal(f.session.phase, 'paused');
        const lateOpen = channel.onopen, lateMessage = channel.onmessage;
        f.session.stop();
        const count = channel.sent.length;
        lateOpen();
        lateMessage({data: JSON.stringify({type: 'startup_telemetry', payload: {success: true}})});
        assert.equal(f.session.phase, 'idle');
        assert.equal(channel.sent.length, count);
      } finally { Peer.prototype.setRemoteDescription = original; f.session.stop(); }
    });
  }
}

test('Microphone controls use the service envelope on open, pause, and resume', async () => {
  const f = fixture();
  microphone(async () => f.stream);
  globalThis.fetch = async () => answer;
  try {
    await f.session.start({mode: 'standard'});
    const channel = f.session.channel;
    f.session.pause();
    f.session.resume();
    assert.deepEqual(channel.sent.map(JSON.parse), ['live', 'muted', 'live'].map(state => ({
      type: 'data_message', data: JSON.stringify({type: 'track_state', payload: {type: 'track_state', track_id: 'microphone', media_type: 'audio', media_source: 'microphone', state}}),
    })));
    channel.send = () => { throw new Error('secret transport error'); };
    f.session.pause();
    assert.equal(f.session.phase, 'idle');
    assert.equal(f.track.stopped, true);
    assert.equal(channel.closed, true);
    assert.ok(!JSON.stringify(f.states).includes('secret'));
  } finally { f.session.stop(); }
});

test('Initial disconnect and reconnect preserve the startup deadline until backend readiness', async () => {
  const f = fixture();
  microphone(async () => f.stream);
  globalThis.fetch = async () => answer;
  try {
    await f.session.start({mode: 'wingman'});
    const peer = f.session.peer, deadline = f.session.timer;
    assert.notEqual(deadline, null);
    peer.connectionState = 'disconnected';
    peer.onconnectionstatechange();
    assert.equal(f.session.timer, deadline);
    assert.notEqual(f.session.recoveryTimer, null);
    peer.connectionState = 'connected';
    peer.onconnectionstatechange();
    assert.equal(f.session.timer, deadline);
    assert.equal(f.session.recoveryTimer, null);
    assert.equal(f.session.phase, 'starting');
    f.session.channel.onmessage({data: JSON.stringify({type: 'startup_telemetry', payload: {success: true}})});
    f.session.pause();
    peer.connectionState = 'disconnected';
    peer.onconnectionstatechange();
    assert.notEqual(f.session.recoveryTimer, null);
    peer.connectionState = 'connected';
    peer.onconnectionstatechange();
    assert.equal(f.session.recoveryTimer, null);
    assert.equal(f.session.phase, 'paused');
  } finally { f.session.stop(); }
});

test('An initially muted microphone is announced as muted and closed channels are never sent to', async () => {
  const f = fixture();
  f.track.enabled = false;
  microphone(async () => f.stream);
  globalThis.fetch = async () => answer;
  try {
    await f.session.start({mode: 'standard'});
    const channel = f.session.channel;
    assert.equal(JSON.parse(JSON.parse(channel.sent[0]).data).payload.state, 'muted');
    channel.readyState = 'closed';
    f.session.pause();
    assert.equal(channel.sent.length, 1);
    assert.equal(f.session.phase, 'idle');
    assert.equal(f.track.stopped, true);
  } finally { f.session.stop(); }
});

test('Signaling posts the offer as JSON to the public realtime route with the local API key', async () => {
  const f = fixture(), requests = [];
  microphone(async () => f.stream);
  globalThis.fetch = async (url, init) => { requests.push({url, init}); return answer; };
  try {
    await f.session.start({mode: 'standard', voice: 'cove'}, ' local-key ');
    assert.equal(requests.length, 1);
    assert.equal(requests[0].url, '/v1/realtime/sessions');
    assert.equal(requests[0].init.method, 'POST');
    assert.deepEqual(requests[0].init.headers, {'Content-Type': 'application/json', Authorization: 'Bearer local-key'});
    assert.deepEqual(JSON.parse(requests[0].init.body), {mode: 'standard', voice: 'cove', sdp: 'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n'});
    assert.equal(f.session.sessionId, 'session-id');
  } finally { f.session.stop(); }
});

test('A failed signaling response surfaces the server error and releases media', async () => {
  const f = fixture();
  microphone(async () => f.stream);
  globalThis.fetch = async () => ({ok: false, status: 429, json: async () => ({error: {message: 'server is busy; try again shortly', type: 'rate_limit_error'}})});
  await f.session.start({mode: 'standard'});
  assert.equal(f.session.phase, 'idle');
  assert.equal(f.states.at(-1).detail, 'server is busy; try again shortly');
  assert.equal(f.track.stopped, true);
  assert.equal(Peer.instances.at(-1).closed, true);
});
