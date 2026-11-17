// Runs in the call page before its own scripts. Mixes every remote audio track into one 16 kHz mono
// stream and reports it to the server about every 100 ms through the `recapzAudio` binding, as
// base64 of 16-bit little-endian samples. Nothing here sends audio anywhere else.
(() => {
  // Audio worklets exist only in secure contexts (https, localhost); about:blank has none.
  // Only the top page: frames from several iframes would interleave into one broken timeline.
  if (window !== window.top || !window.isSecureContext || window.__recapzCapture) return;
  // What the capture saw, read by the server when the call ends (counts and a word only).
  const stats = (window.__recapzCapture = { peers: 0, tracks: 0, frames: 0, path: 'starting' });
  const FRAME = 1600; // 100 ms at 16 kHz
  const ctx = new AudioContext({ sampleRate: 16000 });
  const mix = ctx.createGain();
  const silent = ctx.createGain(); // the graph needs a path to the destination to be pulled
  silent.gain.value = 0;
  const worklet = `class Capture extends AudioWorkletProcessor {
    constructor() { super(); this.buf = new Int16Array(${FRAME}); this.n = 0; }
    process(inputs) {
      const ch = inputs[0][0];
      if (!ch) return true;
      for (let i = 0; i < ch.length; i++) {
        const v = Math.max(-1, Math.min(1, ch[i]));
        this.buf[this.n++] = v < 0 ? v * 32768 : v * 32767;
        if (this.n === this.buf.length) { this.port.postMessage(this.buf.slice().buffer); this.n = 0; }
      }
      return true;
    }
  }
  registerProcessor('recapz-capture', Capture);`;
  const send = (buf) => {
    stats.frames++;
    window.recapzAudio(btoa(String.fromCharCode(...new Uint8Array(buf))));
  };
  // A worklet is the preferred path. If the page's policy refuses its script, a script processor
  // (no script load, deprecated but present in Chrome) does the same job on the main thread.
  const viaProcessor = (why) => {
    stats.path = 'processor: ' + why;
    const node = ctx.createScriptProcessor(4096, 1, 1);
    let buf = new Int16Array(FRAME), n = 0;
    node.onaudioprocess = (e) => {
      const ch = e.inputBuffer.getChannelData(0);
      for (let i = 0; i < ch.length; i++) {
        const v = Math.max(-1, Math.min(1, ch[i]));
        buf[n++] = v < 0 ? v * 32768 : v * 32767;
        if (n === FRAME) { send(buf.buffer.slice(0)); n = 0; }
      }
    };
    mix.connect(node);
    node.connect(silent).connect(ctx.destination);
    ctx.resume();
  };
  try {
    // A data URL, not a blob URL: on the call page blob URLs come out with an opaque origin
    // (`blob:null/…`) and the worklet refuses to load them.
    ctx.audioWorklet.addModule(`data:text/javascript;base64,${btoa(worklet)}`).then(() => {
      const node = new AudioWorkletNode(ctx, 'recapz-capture');
      node.port.onmessage = (e) => send(e.data);
      mix.connect(node);
      node.connect(silent).connect(ctx.destination);
      ctx.resume();
      stats.path = 'worklet';
    }).catch((e) => viaProcessor(String((e && e.name) || e).slice(0, 40)));
  } catch (e) {
    viaProcessor(String((e && e.name) || e).slice(0, 40));
  }

  const Original = window.RTCPeerConnection;
  window.RTCPeerConnection = function (...args) {
    const pc = new Original(...args);
    stats.peers++;
    pc.addEventListener('track', (e) => {
      if (e.track.kind !== 'audio') return;
      stats.tracks++;
      const stream = new MediaStream([e.track]);
      // Chrome only feeds a remote stream to Web Audio while a media element also holds it.
      const el = new Audio();
      el.muted = true;
      el.srcObject = stream;
      el.play().catch(() => {});
      ctx.createMediaStreamSource(stream).connect(mix);
      ctx.resume();
    });
    return pc;
  };
  window.RTCPeerConnection.prototype = Original.prototype;
  Object.setPrototypeOf(window.RTCPeerConnection, Original);
  if (window.webkitRTCPeerConnection) window.webkitRTCPeerConnection = window.RTCPeerConnection;
})();
