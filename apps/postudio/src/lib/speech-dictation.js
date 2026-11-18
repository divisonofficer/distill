/** @typedef {{ supported: boolean, requested: boolean, listening: boolean, interim: string, message: string, error: boolean }} DictationState */
/** @typedef {{ lang: string, continuous: boolean, interimResults: boolean, onstart?: () => void, onresult?: (event: { results: Array<{ isFinal: boolean, 0?: { transcript: string } }> }) => void, onerror?: (event: { error: string }) => void, onend?: () => void, start: () => void, abort?: () => void }} RecognitionInstance */
/** @typedef {{ state: DictationState, stop: (message?: string) => void }} ActiveController */

const RETRY_DELAYS = [300, 700, 1500, 3000, 5000];
/** @type {ActiveController | null} */
let activeController = null;

/** @param {string} draft @param {string} transcript */
export function appendDictationText(draft, transcript) {
  const sentence = transcript.trim();
  if (!sentence) return draft;
  if (!draft) return sentence;
  return `${draft}${/\s$/.test(draft) ? '' : ' '}${sentence}`;
}

/** @param {string} [message] */
export function stopActiveSpeechDictation(message) {
  activeController?.stop(message);
}

/**
 * @param {{ onFinal: (text: string) => void, onState: (state: DictationState) => void, getRecognition: () => (new () => RecognitionInstance) | undefined, secureContext: boolean, timers?: { setTimeout: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>, clearTimeout: (id: ReturnType<typeof setTimeout>) => void } }} options
 */
export function createSpeechDictationController({ onFinal, onState, getRecognition, secureContext, timers = globalThis }) {
  const recognitionConstructor = getRecognition();
  const supported = Boolean(secureContext && recognitionConstructor);
  let requested = false;
  let generation = 0;
  let runId = 0;
  /** @type {RecognitionInstance | null} */
  let recognition = null;
  /** @type {ReturnType<typeof setTimeout> | null} */
  let restartTimer = null;
  let retryCount = 0;
  let language = 'ko-KR';
  /** @type {DictationState} */
  let state = {
    supported,
    requested: false,
    listening: false,
    interim: '',
    message: !secureContext
      ? '음성 인식에는 HTTPS 또는 localhost 보안 연결이 필요합니다.'
      : supported
        ? '한국어 또는 영어를 선택하고 음성 입력을 시작하세요.'
        : '이 브라우저는 SpeechRecognition을 지원하지 않습니다.',
    error: false
  };

  /** @param {Partial<DictationState>} patch */
  function publish(patch) {
    state = { ...state, ...patch };
    onState({ ...state });
  }

  function stop(message = '음성 입력을 중지했습니다.') {
    requested = false;
    generation++;
    runId++;
    if (restartTimer !== null) timers.clearTimeout(restartTimer);
    restartTimer = null;
    const current = recognition;
    recognition = null;
    if (activeController === controller) activeController = null;
    publish({ requested: false, listening: false, interim: '', message, error: false });
    try { current?.abort?.(); } catch { /* Capture already ended. */ }
  }

  /** @param {string} message */
  function fatal(message) {
    stop(message);
    publish({ error: true });
  }

  /** @param {number} token */
  function scheduleRestart(token) {
    if (!requested || token !== generation || activeController !== controller || restartTimer !== null) return;
    publish({ listening: false, interim: '', message: '음성 인식을 다시 연결하고 있습니다…' });
    const delay = RETRY_DELAYS[Math.min(retryCount, RETRY_DELAYS.length - 1)];
    retryCount++;
    restartTimer = timers.setTimeout(() => {
      restartTimer = null;
      if (requested && token === generation && activeController === controller) begin(token);
    }, delay);
  }

  /** @param {number} token */
  function begin(token) {
    if (!requested || token !== generation || activeController !== controller) return;
    const run = ++runId;
    if (!recognitionConstructor) return;
    let instance;
    try {
      instance = new recognitionConstructor();
      recognition = instance;
      instance.lang = language;
      instance.continuous = true;
      instance.interimResults = true;
      instance.onstart = () => {
        if (token !== generation || run !== runId || !requested || activeController !== controller) return;
        publish({ listening: true, message: '듣는 중입니다. 중지를 누르면 녹음을 끝냅니다.', error: false });
      };
      const committed = new Set();
      instance.onresult = (event) => {
        if (token !== generation || run !== runId || !requested || activeController !== controller) return;
        retryCount = 0;
        let interim = '';
        for (let index = 0; index < event.results.length; index++) {
          const result = event.results[index];
          const transcript = result?.[0]?.transcript ?? '';
          if (result?.isFinal) {
            if (!committed.has(index)) {
              committed.add(index);
              if (transcript.trim()) onFinal(transcript.trim());
            }
          } else {
            interim += transcript;
          }
        }
        publish({ interim });
      };
      instance.onerror = (event) => {
        if (token !== generation || run !== runId || !requested || activeController !== controller) return;
        const code = event.error;
        if (code === 'no-speech' || code === 'aborted') {
          publish({ listening: false, interim: '', message: code === 'no-speech' ? '음성이 들리지 않습니다. 계속 듣고 있습니다…' : '음성 인식이 중단되었습니다.' });
          return;
        }
        const message = code === 'not-allowed' || code === 'service-not-allowed'
          ? '마이크 또는 음성 인식 권한이 거부되었습니다. 브라우저 사이트 권한에서 허용한 뒤 다시 시작하세요.'
          : code === 'audio-capture'
            ? '마이크를 사용할 수 없습니다. 연결과 브라우저 마이크 권한을 확인하세요.'
            : code === 'language-not-supported'
              ? '선택한 언어를 음성 인식 서비스가 지원하지 않습니다. 다른 언어를 선택하세요.'
              : code === 'network'
                ? '브라우저 음성 인식 서비스에 연결할 수 없습니다. 네트워크를 확인한 뒤 다시 시작하세요.'
                : `음성 인식 오류(${code || 'unknown'}). 마이크 권한과 브라우저 음성 인식 지원을 확인하세요.`;
        fatal(message);
      };
      instance.onend = () => {
        if (token !== generation || run !== runId || !requested || activeController !== controller) return;
        runId++;
        recognition = null;
        scheduleRestart(token);
      };
      instance.start();
    } catch {
      fatal('마이크를 시작하지 못했습니다. 보안 연결과 브라우저 마이크 권한을 확인하세요.');
    }
  }

  const controller = {
    get state() { return { ...state }; },
    get language() { return language; },
    start() {
      if (!supported) return false;
      if (requested) return true;
      if (activeController && activeController !== controller) activeController.stop('다른 음성 입력을 시작해 녹음을 중지했습니다.');
      activeController = controller;
      requested = true;
      retryCount = 0;
      const token = ++generation;
      publish({ requested: true, listening: false, interim: '', message: '마이크 권한을 확인하고 있습니다…', error: false });
      begin(token);
      return requested;
    },
    stop,
    /** @param {'ko-KR' | 'en-US'} nextLanguage */
    setLanguage(nextLanguage) {
      if (nextLanguage !== 'ko-KR' && nextLanguage !== 'en-US') return;
      if (nextLanguage === language) return;
      const wasRequested = requested;
      if (wasRequested) stop('언어가 바뀌어 녹음을 중지했습니다. 다시 시작하세요.');
      language = nextLanguage;
      if (!wasRequested) publish({ interim: '', message: `${nextLanguage === 'ko-KR' ? '한국어' : 'English'} 선택됨. 음성 입력을 시작하세요.`, error: false });
    },
    reset() {
      stop('음성 입력을 중지했습니다.');
      publish({ interim: '', error: false });
    }
  };

  onState({ ...state });
  return controller;
}
