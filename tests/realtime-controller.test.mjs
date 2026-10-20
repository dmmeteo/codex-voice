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
  assert.deepEqual(f.get('transcripts').children.map(row => row.children.map(child => child.textContent)), [['You', 'Hello'], ['ChatGPT', 'Hi there']]);
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
