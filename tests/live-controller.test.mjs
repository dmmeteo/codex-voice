import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {mountRealtime, TranscriptLog} from '../web/realtime.js';
import {fakeDocument} from './fake-dom.mjs';
const html = await readFile(new URL('../web/index.html', import.meta.url), 'utf8');
test('GPT-Live requires its own privacy acknowledgement and never claims Save off', () => {
  const document = fakeDocument(html);
  let session;
  class Session {
    constructor({onState}) { this.onState = onState; this.phase = 'idle'; this.offers = []; session = this; }
    setState(phase, detail = '') { this.phase = phase; this.onState({phase, detail}); }
    start(options) { this.offers.push(options); }
  }
  mountRealtime(document, {apiKey: () => '', Session});
  const get = id => document.getElementById(id);
  get('mode').value = 'gpt_live';
  get('mode').listeners.change();
  assert.equal(get('start').disabled, true);
  assert.match(get('save-help').textContent, /not verified/);
  assert.equal(get('save-conversation').disabled, true);
  get('live-privacy').checked = true;
  get('live-privacy').listeners.change();
  assert.equal(get('start').disabled, false);
  get('voice').value = 'glimmer';
  get('start').listeners.click();
  assert.equal(session.offers[0].voice, 'sol');
  assert.equal(session.offers[0].acknowledge_upstream_privacy, true);
  assert.equal('history_and_training_disabled' in session.offers[0], false);
  get('mode').value = 'standard';
  get('mode').listeners.change();
  assert.equal(get('save-conversation').disabled, false);
  assert.equal(get('save-conversation').checked, false);
});
test('Live transcript deltas and turn finality form separate speaker turns', () => {
  const log = new TranscriptLog();
  log.receive({type:'live_transcript', role:'user', text:'Hi', append:true, final:false});
  log.receive({type:'live_transcript', role:'user', text:' there', append:true, final:false});
  log.receive({type:'live_transcript', role:'user', text:'Hi there.', append:false, final:true});
  log.receive({type:'live_transcript', role:'assistant', text:'Hello.', append:false, final:true});
  log.receive({type:'live_transcript', role:'user', text:'Next', append:true, final:false});
  assert.deepEqual([...log.entries.values()], [{role:'user',text:'Hi there.'},{role:'assistant',text:'Hello.'},{role:'user',text:'Next'}]);
});
