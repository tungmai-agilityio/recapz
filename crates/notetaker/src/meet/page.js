// Everything the notetaker knows about the Meet call page lives in this file: one table of lookups
// (ARIA roles and visible labels, never class names) and the few actions built on it. Every lookup
// that fails returns a named step, so a page change shows up as `meet_ui_changed: <step>`.
// These labels are our model of the page; only the live check proves they match it.
(() => {
  if (window !== window.top || window.__recapzMeet) return;
  const S = {
    // Vietnamese labels come from a Vietnamese-language account's call page. "Tham gia ngay" and
    // "Yêu cầu tham gia" (join, ask to join) and the camera/microphone "Tắt" labels are not yet seen
    // on a live page; check them on the next live call.
    join: { role: 'button', name: /^(join now|ask to join|tham gia ngay|yêu cầu tham gia)$/i },
    cameraOff: { role: 'button', name: /^(turn off camera|tắt máy ảnh)$/i },
    microphoneOff: { role: 'button', name: /^(turn off microphone|tắt micrô)$/i },
    leave: { role: 'button', name: /^(leave call|rời khỏi cuộc gọi)$/i },
    chatButton: { role: 'button', name: /^(chat with everyone|trò chuyện với mọi người)$/i },
    chatInput: { role: 'textbox', name: /message/i },
    chatSend: { role: 'button', name: /^send( a)? message$/i },
    chatLog: { role: 'list', name: /^chat messages$/i },
    signIn: { role: 'link', name: /^sign in$/i },
    lobbyText: /asking to be let in|waiting for (the host|someone) to let you in|someone will let you in soon|đợi cho đến khi người tổ chức cuộc họp đưa bạn vào cuộc gọi/i,
    deniedText: /denied your request|you can.?t join this call/i,
    rejoin: { role: 'button', name: /^rejoin$/i },
    participantCount: /^(\d+) participants?$/i,
    speaking: /^(.+), speaking$/i,
    removedText: /you.?ve been removed from the (call|meeting)/i,
    endedText: /the (call|meeting) has ended|you left the (call|meeting)/i,
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
      if (S.removedText.test(text())) return 'removed';
      if (S.endedText.test(text()) || find(S.rejoin)) return 'ended';
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
    // Everything the follow loop needs in one round trip: phase, participant count, who is
    // marked as speaking, and the chat log (with our own messages flagged).
    observe() {
      const count = [...document.querySelectorAll('[aria-label]')]
        .map((e) => S.participantCount.exec(e.getAttribute('aria-label')))
        .find((m) => m);
      const speaker = [...document.querySelectorAll('[aria-label]')]
        .map((e) => S.speaking.exec(e.getAttribute('aria-label')))
        .find((m) => m);
      const log = find(S.chatLog);
      const messages = log
        ? [...log.children].map((m) => {
            // Our own lines start "You:"; a participant named "You Li" must not look like us.
            const own = /^you:\s*/i.exec(m.textContent);
            return { text: own ? m.textContent.slice(own[0].length) : m.textContent, self: !!own };
          })
        : [];
      return JSON.stringify({
        state: this.state(),
        count: count ? Number(count[1]) : null,
        speaker: speaker ? speaker[1] : null,
        messages,
      });
    },
    // For the debug dump only: what the page offers right now, so a changed page can be fixed.
    snapshot() {
      const controls = [...document.querySelectorAll('button,a,textarea,input,select,[role]')]
        .slice(0, 80)
        .map((el) => ({
          tag: el.tagName.toLowerCase(),
          role: roleOf(el),
          name: nameOf(el).slice(0, 80),
          disabled: !!disabled(el),
        }));
      return JSON.stringify({
        path: location.pathname,
        title: document.title,
        controls,
        text: text().slice(0, 600),
      });
    },
    leave() {
      const b = find(S.leave);
      if (!b) return fail('leave_button');
      b.click();
      return ok();
    },
    // How many chat messages contain `prefix`; posting must raise it.
    chatCount(prefix) {
      const log = find(S.chatLog);
      return log ? [...log.children].filter((m) => m.textContent.includes(prefix)).length : 0;
    },
  };
})();
