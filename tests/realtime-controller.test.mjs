import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {mountRealtime} from '../web/realtime.js';
import {fakeDocument} from './fake-dom.mjs';

const html = await readFile(new URL('../web/index.html', import.meta.url), 'utf8');

function fixture() {
  const document = fakeDocument(html);
  let session;
  class Session {
    constructor({onState, onEvent}) { this.onState = onState; this.onEvent = onEvent; this.phase = 'idle'; this.offers = []; this.keys = []; session = this; }
    setState(phase, detail = '') { this.phase = phase; this.onState({phase, detail}); }
    start(options, apiKey) { this.offers.push(options); this.keys.push(apiKey); this.setState('starting'); }
    stop() { this.setState('idle'); }
  }
  mountRealtime(document, {apiKey: () => document.getElementById('api-key').value, Session});
  return {get: id => document.getElementById(id), session};
}

test('Save conversation is an accessible switch that starts unchecked', () => {
  const input = html.match(/<input[^>]+id="save-conversation"[^>]*>/)?.[0];
  assert.ok(input);
  assert.match(input, /role="switch"/);
  assert.match(input, /aria-label="Save conversation"/);
  assert.doesNotMatch(input, /\schecked(?:\s|>)/);
  assert.match(html, /ChatGPT history/);
});

test('Defaults are Standard, Cove, transcripts on, and history off', () => {
  const f = fixture();
  assert.equal(f.get('mode').value, 'standard');
  assert.equal(f.get('voice').value, 'cove');
  f.get('start').listeners.click();
  assert.equal(f.session.offers[0].mode, 'standard');
  assert.equal(f.session.offers[0].voice, 'cove');
  assert.equal(f.session.offers[0].enable_message_streaming, true);
  assert.equal(f.session.offers[0].history_and_training_disabled, true);
  assert.equal('language' in f.session.offers[0], false);
});

test('Advanced and Wingman require explicit saving before Start', () => {
  const f = fixture();
  assert.equal(f.get('start').disabled, false);
  for (const mode of ['advanced', 'wingman']) {
    f.get('mode').value = mode;
    f.get('mode').listeners.change();
    assert.equal(f.get('save-conversation').checked, false);
    assert.equal(f.get('start').disabled, true);
    f.get('start').listeners.click();
    assert.equal(f.session.offers.length, 0);
    f.get('save-conversation').checked = true;
    f.get('save-conversation').listeners.change();
    assert.equal(f.get('start').disabled, false);
    f.get('save-conversation').checked = false;
    f.get('save-conversation').listeners.change();
    assert.equal(f.get('start').disabled, true);
  }
  f.get('mode').value = 'standard';
  f.get('mode').listeners.change();
  assert.equal(f.get('start').disabled, false);
});

for (const save of [false, true]) {
  test(`Start sends the inverse save choice and locks settings while starting, active, and paused (${save})`, () => {
    const f = fixture();
    f.get('api-key').value = 'local-key';
    f.get('save-conversation').checked = save;
    f.get('save-conversation').listeners.change();
    f.get('start').listeners.click();
    assert.equal(f.session.offers[0].history_and_training_disabled, !save);
    assert.equal(f.session.offers[0].enable_message_streaming, true);
    assert.deepEqual(f.session.keys, ['local-key']);
    for (const phase of ['starting', 'active', 'paused']) {
      f.session.setState(phase);
      assert.equal(f.get('start').disabled, true);
      assert.equal(f.get('save-conversation').disabled, true);
    }
    f.session.stop();
    assert.equal(f.get('start').disabled, false);
    assert.equal(f.get('save-conversation').disabled, false);
    assert.equal(f.get('save-conversation').checked, save);
  });
}

test('Data channel events fill the Transcript and Events panes; Clear empties only Events', () => {
  const f = fixture();
  f.get('start').listeners.click();
  f.session.onEvent({type: 'full_chat_message', payload: {message: {id: 'u1', author: {role: 'user'}, content: {parts: ['Hello']}}}});
  f.session.onEvent({type: 'full_chat_message', payload: {message: {id: 'a1', author: {role: 'assistant'}, content: {parts: ['Hi there']}, metadata: {resolved_model_slug: 'reported-model'}}}});
  assert.deepEqual(f.get('transcripts').children.map(row => row.children.map(child => child.textContent)), [['ChatGPT', 'Hi there'], ['You', 'Hello']]);
  assert.equal(f.get('transcript-empty').hidden, true);
  assert.equal(f.get('reported-model').textContent, 'reported-model');
  assert.equal(f.get('events').children.length, 2);
  assert.match(f.get('events').children[0].textContent, /full_chat_message/);
  for (let i = 0; i < 200; i++) f.session.onEvent({type: 'tick', payload: {i}});
  assert.equal(f.get('events').children.length, 150);
  f.get('clear').listeners.click();
  assert.equal(f.get('events').children.length, 0);
  assert.equal(f.get('transcripts').children.length, 2);
});

test('A disabled Start states the history requirement in the status line', () => {
  const f = fixture();
  assert.equal(f.get('status').textContent, 'Ready. Start to allow microphone access.');
  f.get('mode').value = 'advanced';
  f.get('mode').listeners.change();
  assert.equal(f.get('start').disabled, true);
  assert.equal(f.get('status').textContent, 'Advanced voice requires Save conversation. Turn it on, or choose Standard.');
  f.get('save-conversation').checked = true;
  f.get('save-conversation').listeners.change();
  assert.equal(f.get('status').textContent, 'Ready. Start to allow microphone access.');
});

const message = (id, role, text) => ({id, author: {role}, content: {parts: [text]}});
const rows = f => f.get('transcripts').children.map(row => [row.dataset.role, ...row.children.map(child => child.textContent)]);

test('New utterances appear at the top as speaker-labelled user and assistant bubbles', () => {
  const f = fixture();
  f.get('start').listeners.click();
  f.session.onEvent({type: 'full_chat_message', payload: {message: message('u1', 'user', 'Hello')}});
  f.session.onEvent({type: 'full_chat_message', payload: {message: message('a1', 'assistant', 'Hi there')}});
  f.session.onEvent({type: 'full_chat_message', payload: {message: message('u2', 'user', 'How are you?')}});
  assert.deepEqual(rows(f), [['user', 'You', 'How are you?'], ['assistant', 'ChatGPT', 'Hi there'], ['user', 'You', 'Hello']]);
  assert.deepEqual(f.get('transcripts').children.map(row => row.tagName), ['ARTICLE', 'ARTICLE', 'ARTICLE']);
});

test('Every rendered speaker role has a bubble style', async () => {
  const css = await readFile(new URL('../web/styles.css', import.meta.url), 'utf8');
  for (const role of ['user', 'assistant']) assert.match(css, new RegExp(`#transcripts article\\[data-role="?${role}"?\\]`));
});

test('Streaming deltas and the final snapshot edit one utterance in place without reordering', () => {
  const f = fixture();
  f.get('start').listeners.click();
  f.session.onEvent({type: 'full_chat_message', payload: {message: message('u1', 'user', 'Hello')}});
  f.session.onEvent({type: 'chat_message_delta', payload: {delta: {c: 0, p: '', o: 'add', v: {message: message('a1', 'assistant', 'Hi')}}}});
  const reply = f.get('transcripts').children[0];
  f.get('transcripts').scrollTop = 120;
  f.session.onEvent({type: 'chat_message_delta', payload: {delta: {p: '/message/content/parts/0', o: 'append', v: ' there'}}});
  assert.deepEqual(rows(f), [['assistant', 'ChatGPT', 'Hi there'], ['user', 'You', 'Hello']]);
  f.session.onEvent({type: 'full_chat_message', payload: {message: message('u2', 'user', 'Thanks')}});
  f.session.onEvent({type: 'chat_message_delta', payload: {delta: {p: '/message/content/parts/0', o: 'append', v: ', friend'}}});
  f.session.onEvent({type: 'streaming_message_update', payload: {request: null, response: {message: message('a1', 'assistant', 'Hi there, friend.')}}});
  assert.deepEqual(rows(f), [['user', 'You', 'Thanks'], ['assistant', 'ChatGPT', 'Hi there, friend.'], ['user', 'You', 'Hello']]);
  assert.equal(f.get('transcripts').children[1], reply);
  assert.equal(f.get('transcripts').scrollTop, 120);
});

test('Transcript keeps the newest 100 utterances and Start clears it', () => {
  const f = fixture();
  f.get('start').listeners.click();
  for (let i = 0; i < 105; i++) f.session.onEvent({type: 'full_chat_message', payload: {message: message(`m${i}`, 'user', `line ${i}`)}});
  const list = f.get('transcripts').children;
  assert.equal(list.length, 100);
  assert.equal(list[0].children[1].textContent, 'line 104');
  assert.equal(list[99].children[1].textContent, 'line 5');
  f.session.stop();
  f.get('start').listeners.click();
  assert.equal(f.get('transcripts').children.length, 0);
  assert.equal(f.get('transcript-empty').hidden, false);
});

test('Events show a type heading over indented JSON in a PRE block, newest first and capped', () => {
  const f = fixture();
  f.session.onEvent({type: 'connection_state', payload: {state: 'connected', detail: {attempt: 1}}});
  const [heading, body] = f.get('events').children[0].children;
  assert.match(heading.textContent, /\sconnection_state$/);
  assert.equal(body.tagName, 'PRE');
  assert.equal(body.textContent, '{\n  "state": "connected",\n  "detail": {\n    "attempt": 1\n  }\n}');
  for (let i = 0; i < 200; i++) f.session.onEvent({type: 'tick', payload: {i}});
  const list = f.get('events').children;
  assert.equal(list.length, 150);
  assert.equal(list[0].children[1].textContent, '{\n  "i": 199\n}');
  assert.equal(list[149].children[1].textContent, '{\n  "i": 50\n}');
});

test('Markup-looking event and transcript text renders as literal text', () => {
  const f = fixture();
  f.get('start').listeners.click();
  const unsafe = '</pre><script>alert(1)</script><img src=x onerror=alert(2)>';
  f.session.onEvent({type: 'full_chat_message', payload: {message: message('u1', 'user', unsafe)}});
  const pre = f.get('events').children[0].children[1];
  assert.equal(pre.tagName, 'PRE');
  assert.equal(pre.children.length, 0);
  assert.equal(pre.textContent, `{\n  "message": {\n    "id": "u1",\n    "author": {\n      "role": "user"\n    },\n    "content": {\n      "parts": [\n        "${unsafe}"\n      ]\n    }\n  }\n}`);
  assert.deepEqual(rows(f), [['user', 'You', unsafe]]);
});
