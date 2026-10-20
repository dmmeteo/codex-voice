import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {TranscriptionSession, mountTranscriptions} from '../web/transcriptions.js';
import {fakeDocument} from './fake-dom.mjs';

function deferred() { let resolve, reject; const promise = new Promise((res, rej) => { resolve = res; reject = rej; }); return {promise, resolve, reject}; }
const ok = text => ({ok: true, status: 200, json: async () => ({text})});

class Recorder {
  static instances = [];
  constructor(stream) { this.stream = stream; this.state = 'inactive'; this.mimeType = 'audio/webm;codecs=opus'; Recorder.instances.push(this); }
  start() { this.state = 'recording'; }
  stop() { this.state = 'inactive'; this.ondataavailable?.({data: new Blob(this.chunks ?? ['opus-bytes'], {type: 'audio/webm'})}); this.onstop?.(); }
}

function fixture({fetch, getUserMedia} = {}) {
  const track = {stopped: false, stop() { this.stopped = true; }};
  const stream = {getTracks: () => [track]};
  const states = [], texts = [], requests = [];
  const session = new TranscriptionSession({
    onState: state => states.push(state),
    onText: text => texts.push(text),
    fetch: fetch ?? (async (url, init) => { requests.push({url, init}); return ok('fixture transcript'); }),
    getUserMedia: getUserMedia ?? (async () => stream),
    Recorder,
  });
  return {session, track, stream, states, texts, requests};
}

test('A file is posted as multipart with language, json format, and the local API key', async () => {
  const f = fixture();
  const file = new File(['RIFF....WAVE'], 'clip.wav', {type: 'audio/wav'});
  await f.session.transcribeFile(file, {language: ' uk ', apiKey: ' local-key '});
  assert.equal(f.requests.length, 1);
  const {url, init} = f.requests[0];
  assert.equal(url, '/v1/audio/transcriptions');
  assert.equal(init.method, 'POST');
  assert.deepEqual(init.headers, {Authorization: 'Bearer local-key'});
  assert.equal(init.body.get('file').name, 'clip.wav');
  assert.equal(await init.body.get('file').text(), 'RIFF....WAVE');
  assert.equal(init.body.get('language'), 'uk');
  assert.equal(init.body.get('response_format'), 'json');
  assert.deepEqual(f.texts, ['fixture transcript']);
  assert.deepEqual(f.states.map(s => s.phase), ['uploading', 'idle']);
  assert.equal(f.states.at(-1).detail, 'Done.');
});

test('Without a key or language only the file and format are sent', async () => {
  const f = fixture();
  await f.session.transcribeFile(new File(['x'], 'a.webm'), {});
  const {init} = f.requests[0];
  assert.deepEqual(init.headers, {});
  assert.deepEqual([...init.body.keys()], ['file', 'response_format']);
});

test('Server errors are shown verbatim and leave the session ready', async () => {
  const f = fixture({fetch: async () => ({ok: false, status: 413, json: async () => ({error: {message: 'multipart body exceeds the configured upload limit', type: 'invalid_request_error'}})})});
  await f.session.transcribeFile(new File(['x'], 'a.wav'), {});
  assert.equal(f.session.phase, 'idle');
  assert.equal(f.states.at(-1).detail, 'multipart body exceeds the configured upload limit');
  assert.deepEqual(f.texts, []);
  const g = fixture({fetch: async () => ({ok: false, status: 502, json: async () => { throw new SyntaxError('bad'); }})});
  await g.session.transcribeFile(new File(['x'], 'a.wav'), {});
  assert.equal(g.states.at(-1).detail, 'Transcription failed (502).');
});

test('Choosing nothing reports it without a request', async () => {
  const f = fixture();
  await f.session.transcribeFile(undefined, {});
  assert.equal(f.requests.length, 0);
  assert.equal(f.states.at(-1).detail, 'Choose an audio file first.');
});

test('Recording releases the microphone on Stop and uploads the recording', async () => {
  const upload = deferred();
  const f = fixture({fetch: async (url, init) => { upload.resolve(init); return ok('spoken words'); }});
  await f.session.startRecording({language: 'en', apiKey: ''});
  assert.equal(f.session.phase, 'recording');
  assert.equal(f.track.stopped, false);
  f.session.stop();
  assert.equal(f.track.stopped, true);
  const init = await upload.promise;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(init.body.get('file').name, 'recording.webm');
  assert.equal(init.body.get('file').type, 'audio/webm');
  assert.equal(await init.body.get('file').text(), 'opus-bytes');
  assert.equal(init.body.get('language'), 'en');
  assert.deepEqual(f.texts, ['spoken words']);
  assert.deepEqual(f.states.map(s => s.phase), ['requesting', 'recording', 'uploading', 'uploading', 'idle']);
});

test('Stop during the permission prompt releases a microphone granted later', async () => {
  const permission = deferred();
  const f = fixture({getUserMedia: () => permission.promise});
  const started = f.session.startRecording({});
  assert.equal(f.session.phase, 'requesting');
  f.session.stop();
  assert.equal(f.session.phase, 'idle');
  permission.resolve(f.stream);
  await started;
  assert.equal(f.track.stopped, true);
  assert.equal(f.session.phase, 'idle');
  assert.equal(f.session.recorder, null);
});

test('Stop during upload aborts it, ignores the late result, and a new transcription works', async () => {
  const response = deferred(), requested = deferred();
  let signal;
  const f = fixture({fetch: (url, init) => { signal = init.signal; requested.resolve(); return response.promise; }});
  const pending = f.session.transcribeFile(new File(['x'], 'a.wav'), {});
  await requested.promise;
  f.session.stop();
  assert.equal(signal.aborted, true);
  assert.equal(f.session.phase, 'idle');
  response.resolve(ok('late text'));
  await pending;
  assert.deepEqual(f.texts, []);
  assert.equal(f.states.at(-1).detail, 'Stopped.');
  f.session.fetch = async () => ok('second');
  await f.session.transcribeFile(new File(['x'], 'b.wav'), {});
  assert.deepEqual(f.texts, ['second']);
});

test('Denied, unsupported, empty, and failed recordings end idle with the microphone released', async () => {
  const denied = fixture({getUserMedia: async () => { throw Object.assign(new Error('no'), {name: 'NotAllowedError'}); }});
  await denied.session.startRecording({});
  assert.equal(denied.states.at(-1).detail, 'Microphone access was denied. Allow access and try again.');

  const unsupported = fixture();
  unsupported.session.Recorder = undefined;
  await unsupported.session.startRecording({});
  assert.equal(unsupported.states.at(-1).detail, 'Recording needs a browser with microphone access on localhost or HTTPS.');

  const empty = fixture();
  await empty.session.startRecording({});
  Recorder.instances.at(-1).chunks = [];
  empty.session.stop();
  assert.equal(empty.session.phase, 'idle');
  assert.equal(empty.states.at(-1).detail, 'Nothing was recorded.');
  assert.equal(empty.track.stopped, true);
  assert.equal(empty.requests.length, 0);

  const failed = fixture();
  await failed.session.startRecording({});
  Recorder.instances.at(-1).onerror();
  assert.equal(failed.session.phase, 'idle');
  assert.equal(failed.states.at(-1).detail, 'Recording failed. Try again.');
  assert.equal(failed.track.stopped, true);
  assert.equal(Recorder.instances.at(-1).state, 'inactive');
});

test('The tab enables only the controls valid for each phase', async () => {
  const html = await readFile(new URL('../web/index.html', import.meta.url), 'utf8');
  const document = fakeDocument(html);
  const $ = id => document.getElementById(id);
  let session;
  class Session {
    constructor({onState, onText}) { this.onState = onState; this.onText = onText; this.phase = 'idle'; this.calls = []; session = this; }
    setState(phase, detail = '') { this.phase = phase; this.onState({phase, detail}); }
    transcribeFile(file, options) { this.calls.push(['file', file, options]); }
    startRecording(options) { this.calls.push(['record', options]); }
    stop() { this.calls.push(['stop']); }
  }
  mountTranscriptions(document, {apiKey: () => 'local-key', Session});
  assert.equal($('transcribe-file').disabled, true);
  assert.equal($('record').disabled, false);
  assert.equal($('record-stop').disabled, true);
  $('transcription-file').files = [{name: 'a.wav'}];
  $('transcription-file').listeners.change();
  assert.equal($('transcribe-file').disabled, false);
  $('transcription-language').value = 'uk';
  $('transcribe-file').listeners.click();
  assert.deepEqual(session.calls[0], ['file', {name: 'a.wav'}, {language: 'uk', apiKey: 'local-key'}]);
  for (const phase of ['requesting', 'recording', 'uploading']) {
    session.setState(phase, phase);
    assert.equal($('transcribe-file').disabled, true);
    assert.equal($('record').disabled, true);
    assert.equal($('record-stop').disabled, false);
    assert.equal($('transcription-file').disabled, true);
    assert.equal($('transcription-status').textContent, phase);
  }
  assert.equal($('record-stop').textContent, 'Stop');
  session.setState('recording');
  assert.equal($('record-stop').textContent, 'Stop and transcribe');
  $('record-stop').listeners.click();
  assert.deepEqual(session.calls.at(-1), ['stop']);
  session.onText('result text');
  assert.equal($('transcription-text').textContent, 'result text');
  session.setState('idle');
  assert.equal($('record').disabled, false);
});
