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
    // "tin nhắn" (message) and "Gửi tin nhắn" (send message) are our reading of the Vietnamese page.
    chatInput: { role: 'textbox', name: /message|tin nhắn/i },
    chatSend: { role: 'button', name: /^(send( a)? message|gửi tin nhắn)$/i },
    chatLog: { role: 'list', name: /^chat messages$/i },
    signIn: { role: 'link', name: /^sign in$/i },
    lobbyText: /asking to be let in|waiting for (the host|someone) to let you in|someone will let you in soon|please wait until a meeting host brings you into the call|đợi cho đến khi người tổ chức cuộc họp đưa bạn vào cuộc gọi/i,
    deniedText: /denied your request|you can.?t join this (video )?call/i,
    rejoin: { role: 'button', name: /^rejoin$/i },
    participantCount: /^(\d+) participants?$/i,
    speaking: /^(.+), speaking$/i,
    removedText: /you.?ve been removed from the (call|meeting)/i,
    endedText: /the (call|meeting) has ended|you left the (call|meeting)/i,
    // The Vietnamese words are our guess at the page, not yet seen live.
    chatOffText: /chat (is|has been) turned off|can.?t send messages|trò chuyện (đã |đang )?(bị )?tắt|không thể gửi tin nhắn/i,
  };
  const roleOf = (el) =>
    el.getAttribute('role') || { BUTTON: 'button', A: 'link', TEXTAREA: 'textbox', INPUT: 'textbox' }[el.tagName] || '';
  // A text box can be named by a label element or only by its placeholder.
  const labelledBy = (el) =>
    (el.getAttribute('aria-labelledby') || '')
      .split(/\s+/)
      .map((id) => (id && document.getElementById(id)?.textContent) || '')
      .join(' ')
      .trim();
  // Only text boxes read them, so button and link names stay as they were.
  const nameOf = (el) =>
    (
      el.getAttribute('aria-label') ||
      (roleOf(el) === 'textbox'
        ? labelledBy(el) || el.getAttribute('placeholder') || el.getAttribute('aria-placeholder')
        : '') ||
      el.textContent ||
      ''
    ).trim();
  const find = (s) =>
    [...document.querySelectorAll('button,a,textarea,input,ul,ol,[role]')].find(
      (el) => roleOf(el) === s.role && s.name.test(nameOf(el)),
    );
  // ponytail: with no box named like a message box, the page's only text box is taken as it (the
  // call page has none other once admitted); with two unnamed boxes this still fails.
  const chatBox = () => {
    const boxes = [...document.querySelectorAll('textarea,input,[role="textbox"]')].filter(
      (el) => roleOf(el) === 'textbox' && !/^(hidden|checkbox|radio|button|submit)$/.test(el.type || ''),
    );
    return find(S.chatInput) || (boxes.length === 1 ? boxes[0] : null);
  };
  const text = () => document.body.innerText;
  const ok = (extra) => JSON.stringify({ ok: true, ...extra });
  const fail = (step) => JSON.stringify({ ok: false, step });
  const disabled = (el) => el.disabled || el.getAttribute('aria-disabled') === 'true';

  window.__recapzMeet = {
    // The page's current phase, in the order a call goes through them.
    state() {
      if (S.removedText.test(text())) return 'removed';
      if (S.endedText.test(text()) || find(S.rejoin)) return 'ended';
      // The lobby shows the leave button too, so its waiting text must win over it.
      if (S.lobbyText.test(text())) return 'lobby';
      if (find(S.leave)) return 'admitted';
      if (S.deniedText.test(text())) return 'denied';
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
      // The panel is drawn after the click, so the caller waits for the input (`focusChatInput`).
      if (!chatBox()) b.click();
      return ok();
    },
    focusChatInput() {
      const i = chatBox();
      // Chat that the host turned off shows its notice in the panel instead of a box.
      if (!i) return fail(S.chatOffText.test(text()) ? 'chat_disabled' : 'chat_input');
      i.focus();
      return ok();
    },
    clickSend() {
      const b = find(S.chatSend);
      if (!b) return fail('chat_send');
      // Off until the page has seen the text, and nothing to send once the box is empty. `clicked`
      // tells the caller whether this call sent it, so that it clicks only once.
      const i = chatBox();
      const clicked = !disabled(b) && !!i && !!i.value;
      if (clicked) b.click();
      return ok({ clicked });
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
      // Tooltips repeat every button's name and would push the chat panel out of the list.
      const controls = [...document.querySelectorAll('button,a,textarea,input,select,[role]')]
        .filter((el) => roleOf(el) !== 'tooltip')
        .slice(0, 120)
        .map((el) => ({
          tag: el.tagName.toLowerCase(),
          role: roleOf(el),
          name: nameOf(el).slice(0, 80),
          disabled: !!disabled(el),
        }));
      // The chat panel's tags, roles and attribute names with every text replaced by its length, so
      // the dump shows how messages are built without keeping what was said.
      const shape = (el, depth = 0) => {
        const tag = el.tagName.toLowerCase();
        const attrs = [...el.attributes]
          .filter((a) => a.name !== 'class' && a.name !== 'style')
          .map((a) => (a.name === 'role' || a.name === 'aria-label' ? `${a.name}="${a.value.slice(0, 40)}"` : a.name));
        const kids = depth > 9 ? '' : [...el.childNodes]
          .map((n) => (n.nodeType === 3 ? (n.data.trim() ? `[${n.data.trim().length}]` : '') : n.nodeType === 1 ? shape(n, depth + 1) : ''))
          .join('');
        return `<${[tag, ...attrs].join(' ')}>${kids}</${tag}>`;
      };
      let panel = chatBox();
      for (let i = 0; panel && i < 6 && panel.parentElement; i++) panel = panel.parentElement;
      return JSON.stringify({
        path: location.pathname,
        title: document.title,
        controls,
        chatShape: panel ? shape(panel).slice(0, 6000) : '',
        text: text().slice(0, 600),
      });
    },
    leave() {
      const b = find(S.leave);
      if (!b) return fail('leave_button');
      b.click();
      return ok();
    },
    // How many times `prefix` shows on the page; posting must raise it. The live chat log has no
    // label to look it up by, so this reads the page text (the input box's text is not part of it).
    // ponytail: another person quoting the notice, or the panel being closed, changes the count; a
    // labelled message list would be exact, once its markup on the live page is known.
    chatCount(prefix) {
      return text().split(prefix).length - 1;
    },
  };
})();
