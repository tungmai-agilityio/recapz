// Everything the notetaker knows about the Meet call page lives in this file: one table of lookups
// (ARIA roles and visible labels, never class names) and the few actions built on it. Every lookup
// that fails returns a named step, so a page change shows up as `meet_ui_changed: <step>`.
// These labels are our model of the page; only the live check proves they match it.
(() => {
  if (window !== window.top || window.__recapzMeet) return;
  const S = {
    join: { role: 'button', name: /^(join now|ask to join)$/i },
    cameraOff: { role: 'button', name: /^turn off camera$/i },
    microphoneOff: { role: 'button', name: /^turn off microphone$/i },
    leave: { role: 'button', name: /^leave call$/i },
    chatButton: { role: 'button', name: /^chat with everyone$/i },
    chatInput: { role: 'textbox', name: /message/i },
    chatSend: { role: 'button', name: /^send( a)? message$/i },
    chatLog: { role: 'list', name: /^chat messages$/i },
    signIn: { role: 'link', name: /^sign in$/i },
    lobbyText: /asking to be let in|waiting for (the host|someone) to let you in|someone will let you in soon/i,
    deniedText: /denied your request|you can.?t join this call/i,
    chatOffText: /chat (is|has been) turned off|can.?t send messages/i,
  };
  const roleOf = (el) =>
    el.getAttribute('role') || { BUTTON: 'button', A: 'link', TEXTAREA: 'textbox', INPUT: 'textbox' }[el.tagName] || '';
  const nameOf = (el) => (el.getAttribute('aria-label') || el.textContent || '').trim();
  const find = (s) =>
    [...document.querySelectorAll('button,a,textarea,input,ul,ol,[role]')].find(
      (el) => roleOf(el) === s.role && s.name.test(nameOf(el)),
    );
  const text = () => document.body.innerText;
  const ok = (extra) => JSON.stringify({ ok: true, ...extra });
  const fail = (step) => JSON.stringify({ ok: false, step });
  const disabled = (el) => el.disabled || el.getAttribute('aria-disabled') === 'true';

  window.__recapzMeet = {
    // The page's current phase, in the order a call goes through them.
    state() {
      if (find(S.leave)) return 'admitted';
      if (S.deniedText.test(text())) return 'denied';
      if (S.lobbyText.test(text())) return 'lobby';
      if (find(S.join)) return 'prejoin';
      if (find(S.signIn)) return 'signed_out';
      return 'unknown';
    },
    // Camera and microphone off before joining; missing toggles mean they are already off.
    prepare() {
      for (const s of [S.cameraOff, S.microphoneOff]) find(s)?.click();
      return ok();
    },
    join() {
      const b = find(S.join);
      if (!b) return fail('join_button');
      b.click();
      return ok();
    },
    openChat() {
      const b = find(S.chatButton);
      if (!b) return fail('chat_button');
      if (disabled(b) || S.chatOffText.test(text())) return fail('chat_disabled');
      if (!find(S.chatInput)) b.click();
      return find(S.chatInput) ? ok() : fail('chat_input');
    },
    focusChatInput() {
      const i = find(S.chatInput);
      if (!i) return fail('chat_input');
      i.focus();
      return ok();
    },
    clickSend() {
      const b = find(S.chatSend);
      if (!b) return fail('chat_send');
      b.click();
      return ok();
    },
    // Whether a message starting with `prefix` is in the chat log.
    chatHas(prefix) {
      const log = find(S.chatLog);
      return !!log && [...log.children].some((m) => m.textContent.includes(prefix));
    },
  };
})();
