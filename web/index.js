import {mountTranscriptions} from './transcriptions.js';
import {mountRealtime} from './realtime.js';

const apiKey = () => document.getElementById('api-key').value;

function bindTabs(list) {
  const tabs = [...list.querySelectorAll('[role=tab]')];
  const select = tab => {
    for (const other of tabs) {
      const selected = other === tab;
      other.setAttribute('aria-selected', String(selected));
      other.tabIndex = selected ? 0 : -1;
      document.getElementById(other.getAttribute('aria-controls')).hidden = !selected;
    }
  };
  for (const tab of tabs) {
    tab.addEventListener('click', () => select(tab));
    tab.addEventListener('keydown', event => {
      const step = {ArrowRight: 1, ArrowLeft: -1}[event.key];
      if (!step) return;
      const next = tabs[(tabs.indexOf(tab) + step + tabs.length) % tabs.length];
      select(next);
      next.focus();
    });
  }
}

for (const list of document.querySelectorAll('[role=tablist]')) bindTabs(list);
const transcriptions = mountTranscriptions(document, {apiKey});
const realtime = mountRealtime(document, {apiKey});
window.addEventListener('pagehide', () => {
  transcriptions.cancel();
  realtime.stop();
});
