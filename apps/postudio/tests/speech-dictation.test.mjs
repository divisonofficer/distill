import test from 'node:test';
import assert from 'node:assert/strict';
import { appendDictationText, createSpeechDictationController } from '../src/lib/speech-dictation.js';

class Clock {
  now = 0;
  nextId = 0;
  tasks = new Map();
  setTimeout = (callback, delay) => {
    const id = ++this.nextId;
    this.tasks.set(id, { callback, at: this.now + delay });
    return id;
  };
  clearTimeout = (id) => this.tasks.delete(id);
  tick(milliseconds) {
    const until = this.now + milliseconds;
    while (true) {
      const due = [...this.tasks].find(([, task]) => task.at <= until);
      if (!due) break;
      const [id, task] = due;
      this.tasks.delete(id);
      this.now = task.at;
      task.callback();
    }
    this.now = until;
  }
  nextDelay() {
    return [...this.tasks.values()][0]?.at - this.now;
  }
}

function harness() {
  const instances = [];
  const commits = [];
  const states = [];
  class Recognition {
    constructor() { instances.push(this); }
    start() { this.started = true; }
    abort() { this.aborted = true; }
  }
  const clock = new Clock();
  const controller = createSpeechDictationController({
    getRecognition: () => Recognition,
    secureContext: true,
    timers: clock,
    onFinal: (value) => commits.push(value),
    onState: (value) => states.push(value)
  });
  return { controller, instances, commits, states, clock };
}

function result(transcript, isFinal) {
  return { 0: { transcript }, isFinal };
}

function recognitionEvent(...results) {
  return { resultIndex: 0, results };
}

test('keeps interim separate and commits each final result exactly once', () => {
  const { controller, instances, commits, states } = harness();
  controller.start();
  const recognition = instances[0];
  assert.equal(recognition.continuous, true);
  assert.equal(recognition.interimResults, true);
  recognition.onstart();
  recognition.onresult(recognitionEvent(result('첫 문장.', false)));
  assert.deepEqual(commits, []);
  assert.equal(states.at(-1).interim, '첫 문장.');
  recognition.onresult(recognitionEvent(result('첫 문장.', true)));
  recognition.onresult(recognitionEvent(result('첫 문장.', true)));
  recognition.onresult(recognitionEvent(result('첫 문장.', true), result('둘째 문장.', true)));
  assert.deepEqual(commits, ['첫 문장.', '둘째 문장.']);
  assert.equal(states.at(-1).interim, '');
});

test('finish uses the final transcript without also inserting the preview', () => {
  const { controller, instances, commits, clock } = harness();
  controller.start();
  const recognition = instances[0];
  recognition.stop = () => {};
  recognition.onresult(recognitionEvent(result('미리보기 문장', false)));
  controller.finish();
  recognition.onresult(recognitionEvent(result('확정 문장', true)));
  recognition.onend();
  clock.tick(2000);
  assert.deepEqual(commits, ['확정 문장']);
  assert.equal(controller.state.requested, false);
  assert.equal(instances.length, 1);
});

test('finish inserts a preview once if the browser never finalizes it', () => {
  const { controller, instances, commits, clock } = harness();
  controller.start();
  const recognition = instances[0];
  recognition.stop = () => {};
  recognition.onresult(recognitionEvent(result('확정 구간', true), result(' 나머지', false)));
  controller.finish();
  clock.tick(1200);
  recognition.onresult(recognitionEvent(result('확정 구간', true), result('늦은 결과', true)));
  assert.deepEqual(commits, ['확정 구간', '나머지']);
  assert.equal(controller.state.interim, '');
  assert.equal(instances.length, 1);
});

test('a spontaneous service end preserves the last unfinalized words once', () => {
  const { controller, instances, commits, clock } = harness();
  controller.start();
  const recognition = instances[0];
  recognition.onresult(recognitionEvent(result('완료', true), result(' 마무리', false)));
  recognition.onerror({ error: 'no-speech' });
  recognition.onend();
  recognition.onresult(recognitionEvent(result('완료', true), result('늦은 확정', true)));
  assert.deepEqual(commits, ['완료', '마무리']);
  assert.equal(controller.state.interim, '');
  clock.tick(300);
  assert.equal(instances.length, 2);
});

test('restarts after service end with delayed bounded backoff', () => {
  const { controller, instances, clock } = harness();
  controller.start();
  instances[0].onend();
  assert.equal(instances.length, 1);
  assert.equal(clock.nextDelay(), 300);
  clock.tick(299);
  assert.equal(instances.length, 1);
  clock.tick(1);
  assert.equal(instances.length, 2);
  instances[1].onend();
  assert.equal(clock.nextDelay(), 700);
  clock.tick(700);
  assert.equal(instances.length, 3);
  instances[2].onend();
  assert.equal(clock.nextDelay(), 1500);
  clock.tick(1500);
  instances[3].onend();
  assert.equal(clock.nextDelay(), 3000);
  clock.tick(3000);
  instances[4].onend();
  assert.equal(clock.nextDelay(), 5000);
  clock.tick(5000);
  instances[5].onend();
  assert.equal(clock.nextDelay(), 5000);
});

test('backoff still grows when service starts but returns no results', () => {
  const { controller, instances, clock } = harness();
  controller.start();
  instances[0].onend();
  clock.tick(300);
  instances[1].onstart();
  instances[1].onend();
  assert.equal(clock.nextDelay(), 700);
});

test('stop discards interim, releases capture, and ignores late events', () => {
  const { controller, instances, commits, clock } = harness();
  controller.start();
  const recognition = instances[0];
  recognition.onresult(recognitionEvent(result('아직 임시 문장', false)));
  controller.stop();
  recognition.onresult(recognitionEvent(result('늦은 결과.', true)));
  recognition.onend();
  clock.tick(10_000);
  assert.deepEqual(commits, []);
  assert.equal(recognition.aborted, true);
  assert.equal(instances.length, 1);
  assert.equal(controller.state.requested, false);
  assert.equal(controller.state.interim, '');
});

test('permission and fatal errors stop retries with actionable status', () => {
  const { controller, instances, clock } = harness();
  controller.start();
  instances[0].onerror({ error: 'not-allowed' });
  instances[0].onend();
  clock.tick(10_000);
  assert.equal(controller.state.requested, false);
  assert.equal(controller.state.error, true);
  assert.match(controller.state.message, /권한/);
  assert.equal(instances.length, 1);
});

test('language change stops old capture, ignores its late result, and reset clears state', () => {
  const { controller, instances, commits } = harness();
  controller.start();
  const oldRecognition = instances[0];
  controller.setLanguage('en-US');
  oldRecognition.onresult(recognitionEvent(result('late sentence', true)));
  controller.start();
  assert.equal(instances[1].lang, 'en-US');
  instances[1].onresult(recognitionEvent(result('interim', false)));
  controller.reset();
  instances[1].onresult(recognitionEvent(result('late final', true)));
  assert.deepEqual(commits, []);
  assert.equal(controller.state.interim, '');
  assert.equal(controller.state.requested, false);
});

test('only one composer owns capture at a time', () => {
  const first = harness();
  const second = harness();
  first.controller.start();
  second.controller.start();
  assert.equal(first.controller.state.requested, false);
  assert.equal(first.instances[0].aborted, true);
  assert.equal(second.controller.state.requested, true);
});

test('append preserves typed draft and separates appended sentences', () => {
  assert.equal(appendDictationText('typed draft', 'spoken sentence.'), 'typed draft spoken sentence.');
  assert.equal(appendDictationText('typed draft\n', 'spoken sentence.'), 'typed draft\nspoken sentence.');
  assert.equal(appendDictationText('', ' spoken sentence. '), 'spoken sentence.');
  assert.equal(appendDictationText('draft', '  '), 'draft');
});

test('unsupported and insecure contexts cannot start recognition', () => {
  const unsupported = createSpeechDictationController({ getRecognition: () => undefined, secureContext: true, onFinal() {}, onState() {} });
  assert.equal(unsupported.state.supported, false);
  assert.equal(unsupported.start(), false);
  const insecure = createSpeechDictationController({ getRecognition: () => class {}, secureContext: false, onFinal() {}, onState() {} });
  assert.equal(insecure.state.supported, false);
  assert.match(insecure.state.message, /HTTPS|localhost/);
});
