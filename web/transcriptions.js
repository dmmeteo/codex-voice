const UNSUPPORTED = 'Recording needs a browser with microphone access on localhost or HTTPS.';
const EXTENSIONS = {'audio/webm': 'webm', 'audio/ogg': 'ogg', 'audio/mp4': 'm4a', 'audio/wav': 'wav', 'audio/mpeg': 'mp3'};

/**
 * One transcription at a time, from a file or a microphone recording.
 * Phases: idle → requesting → recording → uploading → idle. Every exit
 * releases the microphone; results from a cancelled generation are dropped.
 */
export class TranscriptionSession {
  constructor({onState, onText, fetch = (...args) => globalThis.fetch(...args),
    getUserMedia = async constraints => {
      if (!globalThis.navigator?.mediaDevices?.getUserMedia) throw new Error(UNSUPPORTED);
      return globalThis.navigator.mediaDevices.getUserMedia(constraints);
    },
    Recorder = globalThis.MediaRecorder}) {
    this.onState = onState;
    this.onText = onText;
    this.fetch = fetch;
    this.getUserMedia = getUserMedia;
    this.Recorder = Recorder;
    this.phase = 'idle';
    this.generation = 0;
    this.stream = null;
    this.recorder = null;
    this.abort = null;
  }

  setState(phase, detail = '') {
    this.phase = phase;
    this.onState({phase, detail});
  }

  transcribeFile(file, options) {
    if (this.phase !== 'idle') return Promise.resolve();
    if (!file) {
      this.setState('idle', 'Choose an audio file first.');
      return Promise.resolve();
    }
    this.cleanup();
    return this.#upload(file, file.name || 'audio', options);
  }

  async startRecording(options) {
    if (this.phase !== 'idle') return;
    this.cleanup();
    const generation = this.generation;
    this.setState('requesting', 'Requesting microphone access…');
    try {
      if (!this.Recorder) throw new Error(UNSUPPORTED);
      const stream = await this.getUserMedia({audio: true, video: false});
      if (generation !== this.generation) { stream.getTracks().forEach(track => track.stop()); return; }
      this.stream = stream;
      const chunks = [];
      const recorder = new this.Recorder(stream);
      this.recorder = recorder;
      recorder.ondataavailable = ({data}) => { if (data?.size) chunks.push(data); };
      recorder.onerror = () => { if (generation === this.generation) this.fail('Recording failed. Try again.'); };
      recorder.onstop = () => {
        if (generation !== this.generation) return;
        this.recorder = null;
        this.#releaseMicrophone();
        const type = (recorder.mimeType || chunks[0]?.type || 'audio/webm').split(';')[0].trim();
        const blob = new Blob(chunks, {type});
        if (!blob.size) { this.setState('idle', 'Nothing was recorded.'); return; }
        this.#upload(blob, `recording.${EXTENSIONS[type] || 'webm'}`, options);
      };
      recorder.start();
      this.setState('recording', 'Recording. Stop to transcribe.');
    } catch (error) {
      if (generation === this.generation) {
        this.fail(error?.name === 'NotAllowedError'
          ? 'Microphone access was denied. Allow access and try again.'
          : error?.message || 'Could not start recording.');
      }
    }
  }

  /** Ends a recording and uploads it; in any other phase it cancels. */
  stop() {
    if (this.phase !== 'recording') { this.cancel(); return; }
    this.setState('uploading', 'Transcribing…');
    this.recorder.stop();
  }

  cancel(detail = 'Stopped.') {
    if (this.phase === 'idle') return;
    this.cleanup();
    this.setState('idle', detail);
  }

  fail(message) {
    this.cleanup();
    this.setState('idle', message);
  }

  cleanup() {
    this.generation += 1;
    this.abort?.abort();
    this.abort = null;
    if (this.recorder) {
      const recorder = this.recorder;
      recorder.ondataavailable = recorder.onstop = recorder.onerror = null;
      if (recorder.state !== 'inactive') recorder.stop();
    }
    this.recorder = null;
    this.#releaseMicrophone();
  }

  #releaseMicrophone() {
    this.stream?.getTracks().forEach(track => track.stop());
    this.stream = null;
  }

  async #upload(blob, filename, {language = '', apiKey = ''} = {}) {
    const generation = this.generation;
    const abort = new AbortController();
    this.abort = abort;
    this.setState('uploading', 'Transcribing…');
    try {
      const form = new FormData();
      form.append('file', blob, filename);
      if (language.trim()) form.append('language', language.trim());
      form.append('response_format', 'json');
      const headers = apiKey.trim() ? {Authorization: `Bearer ${apiKey.trim()}`} : {};
      const response = await this.fetch('/v1/audio/transcriptions', {method: 'POST', headers, body: form, signal: abort.signal});
      const result = await response.json().catch(() => null);
      if (generation !== this.generation) return;
      if (!response.ok) throw new Error(result?.error?.message || `Transcription failed (${response.status}).`);
      if (typeof result?.text !== 'string') throw new Error('The server returned an unreadable response.');
      this.abort = null;
      this.onText(result.text);
      this.setState('idle', 'Done.');
    } catch (error) {
      if (generation === this.generation) this.fail(error?.message || 'Transcription failed.');
    }
  }
}

/** Binds the Transcriptions tab. `Session` is injectable for tests. */
export function mountTranscriptions(document, {apiKey, Session = TranscriptionSession} = {}) {
  const $ = id => document.getElementById(id);
  const options = () => ({language: $('transcription-language').value, apiKey: apiKey()});
  const render = phase => {
    const idle = phase === 'idle';
    $('transcribe-file').disabled = !idle || !$('transcription-file').files?.length;
    $('record').disabled = !idle;
    $('record-stop').disabled = idle;
    $('record-stop').textContent = phase === 'recording' ? 'Stop and transcribe' : 'Stop';
    $('transcription-file').disabled = !idle;
    $('transcription-language').disabled = !idle;
  };
  const session = new Session({
    onState: ({phase, detail}) => {
      $('transcription-status').textContent = detail;
      render(phase);
    },
    onText: text => { $('transcription-text').textContent = text; },
  });
  $('transcription-file').addEventListener('change', () => render(session.phase));
  $('transcribe-file').addEventListener('click', () => session.transcribeFile($('transcription-file').files?.[0], options()));
  $('record').addEventListener('click', () => session.startRecording(options()));
  $('record-stop').addEventListener('click', () => session.stop());
  render(session.phase);
  return session;
}
