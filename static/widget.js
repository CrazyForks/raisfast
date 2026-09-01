const se = ".rw-root{--rw-brand: #4f46e5;position:fixed;bottom:20px;z-index:2147483000;font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif;font-size:14px;line-height:1.4}.rw-root[data-position=right]{right:20px}.rw-root[data-position=left]{left:20px}.rw-launcher{width:56px;height:56px;border-radius:50%;border:none;background:var(--rw-brand);color:#fff;cursor:pointer;display:flex;align-items:center;justify-content:center;box-shadow:0 4px 16px #00000040;transition:transform .15s ease}.rw-launcher:hover{transform:scale(1.05)}.rw-launcher svg{width:26px;height:26px}.rw-badge{position:absolute;top:-2px;right:-2px;min-width:18px;height:18px;border-radius:9px;background:#ef4444;color:#fff;font-size:11px;font-weight:600;display:flex;align-items:center;justify-content:center;padding:0 4px}.rw-badge[hidden]{display:none}.rw-panel{position:absolute;bottom:72px;width:360px;max-width:calc(100vw - 40px);height:480px;max-height:calc(100vh - 120px);border-radius:16px;background:#fff;box-shadow:0 12px 40px #0003;display:none;flex-direction:column;overflow:hidden;border:1px solid rgba(0,0,0,.08)}.rw-root[data-position=right] .rw-panel{right:0}.rw-root[data-position=left] .rw-panel{left:0}.rw-panel.rw-open{display:flex}.rw-header{display:flex;align-items:center;gap:8px;padding:12px 14px;background:var(--rw-brand);color:#fff}.rw-avatar{width:32px;height:32px;border-radius:50%;object-fit:cover}.rw-title{font-weight:600;flex:1}.rw-conn{font-size:12px;opacity:.85}.rw-conn.rw-conn{display:none}.rw-close{border:none;background:transparent;color:#fff;font-size:20px;cursor:pointer;line-height:1;padding:0 2px}.rw-list{flex:1;overflow-y:auto;padding:12px;display:flex;flex-direction:column;gap:8px;background:#f8fafc}.rw-empty{color:#94a3b8;text-align:center;margin-top:24px;font-size:13px}.rw-msg{display:flex}.rw-msg.rw-out{justify-content:flex-end}.rw-msg.rw-in{justify-content:flex-start}.rw-bubble{max-width:78%;padding:8px 12px;border-radius:14px;word-break:break-word}.rw-msg.rw-out .rw-bubble{background:var(--rw-brand);color:#fff;border-bottom-right-radius:4px}.rw-msg.rw-in .rw-bubble{background:#fff;border:1px solid #e2e8f0;border-bottom-left-radius:4px;color:#0f172a}.rw-bubble.rw-pending{opacity:.6}.rw-bubble.rw-failed{border:1px solid #ef4444}.rw-text a{color:inherit;text-decoration:underline}.rw-typing{padding:4px 14px;font-size:12px;color:#64748b;font-style:italic}.rw-typing[hidden]{display:none}.rw-composer{display:flex;gap:8px;padding:10px;border-top:1px solid #e2e8f0;background:#fff}.rw-composer input{flex:1;border:1px solid #cbd5e1;border-radius:10px;padding:8px 12px;font-size:14px;outline:none}.rw-composer input:focus{border-color:var(--rw-brand)}.rw-send{border:none;border-radius:10px;background:var(--rw-brand);color:#fff;padding:0 14px;font-size:13px;font-weight:600;cursor:pointer}.rw-greeting{position:absolute;bottom:72px;right:0;background:#fff;border:1px solid #e2e8f0;border-radius:10px;padding:8px 12px;box-shadow:0 6px 20px #0000001f;font-size:13px;animation:rwFade .2s ease}.rw-root[data-position=left] .rw-greeting{left:0;right:auto}@keyframes rwFade{0%{opacity:0;transform:translateY(4px)}to{opacity:1;transform:none}}@media(prefers-reduced-motion:reduce){.rw-launcher{transition:none}.rw-greeting{animation:none}}", s = document.currentScript;
var Z;
const O = (Z = s == null ? void 0 : s.dataset.channel) != null ? Z : "";
var ee;
const de = new URL((ee = s == null ? void 0 : s.src) != null ? ee : window.location.href).origin, K = `${de}/api/v1`, D = (e) => `raf.widget.${O}.${e}`;
var te, ne, oe, re, ae;
const S = {
  color: (te = s == null ? void 0 : s.dataset.color) != null ? te : "#4f46e5",
  position: (ne = s == null ? void 0 : s.dataset.position) != null ? ne : "right",
  avatar: (oe = s == null ? void 0 : s.dataset.avatar) != null ? oe : null,
  locale: (re = s == null ? void 0 : s.dataset.locale) != null ? re : document.documentElement.lang || "en",
  greeting: (ae = s == null ? void 0 : s.dataset.greeting) != null ? ae : null
}, H = {
  en: {
    open: "Chat with us",
    placeholder: "Type a message…",
    send: "Send",
    connecting: "Connecting…",
    retry: "Retry",
    unavailable: "Chat is unavailable right now.",
    agentTyping: "Support is typing…",
    botName: "Bot"
  },
  zh: {
    open: "在线咨询",
    placeholder: "输入消息…",
    send: "发送",
    connecting: "连接中…",
    retry: "重试",
    unavailable: "客服暂时不可用。",
    agentTyping: "客服正在输入…",
    botName: "机器人"
  }
}, w = (e) => {
  var t, n, o;
  return (o = (n = ((t = H[S.locale]) != null ? t : H.en)[e]) != null ? n : H.en[e]) != null ? o : e;
}, v = {
  get(e) {
    try {
      return localStorage.getItem(D(e));
    } catch {
      return null;
    }
  },
  set(e, t) {
    try {
      localStorage.setItem(D(e), t);
    } catch {
    }
  },
  del(e) {
    try {
      localStorage.removeItem(D(e));
    } catch {
    }
  }
};
let i = null, W = !1, F = !1, T = 0;
const c = [];
let C = !1, J = !1, Y = !1;
function d(e, t, n) {
  const o = document.createElement(e);
  return t && (o.className = t), n !== void 0 && (o.textContent = n), o;
}
function le(e) {
  const t = e.split(/(https?:\/\/[^\s]+)/g), n = [];
  for (const o of t)
    if (/^https?:\/\//.test(o)) {
      const r = document.createElement("a");
      r.href = o, r.target = "_blank", r.rel = "noopener noreferrer", r.textContent = o, n.push(r);
    } else o && n.push(document.createTextNode(o));
  return n;
}
async function j(e, t = {}) {
  var l;
  const n = {
    "Content-Type": "application/json",
    ...t.headers
  };
  i != null && i.token && (n.Authorization = `Bearer ${i.token}`);
  const o = await fetch(`${K}${e}`, { ...t, headers: n }), r = await o.json().catch(() => ({}));
  if (!o.ok || r.code !== 0)
    throw new Error((l = r.message) != null ? l : `HTTP ${o.status}`);
  return r.data;
}
function q() {
  var e, t;
  return (t = (e = crypto.randomUUID) == null ? void 0 : e.call(crypto)) != null ? t : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
async function ce(e, t) {
  if (!i) throw new Error("no session");
  const n = await fetch(`${K}/ingress/${O}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${i.token}`
    },
    body: JSON.stringify({ id: t, text: e })
  });
  if (!n.ok) throw new Error(`HTTP ${n.status}`);
}
async function pe() {
  var n, o;
  const e = (n = v.get("visitor_id")) != null ? n : q();
  v.set("visitor_id", e);
  const t = await j("/plugins/chat/widget/session", {
    method: "POST",
    body: JSON.stringify({ channel_key: O, visitor_id: e })
  });
  i = {
    token: t.token,
    contactId: t.contact_id,
    conversationId: t.conversation_id,
    visitorId: e,
    greeting: (o = t.greeting) != null ? o : S.greeting
  }, v.set("token", t.token), v.set("conversation_id", t.conversation_id);
}
async function fe(e) {
  var o;
  if (!i) return [];
  const t = new URLSearchParams({ conversation: i.conversationId });
  return e && t.set("since", e), ((o = (await j(`/plugins/chat/widget/messages?${t}`)).items) != null ? o : []).map((r) => {
    var l, a;
    return {
      id: String(r.id),
      role: String((l = r.role) != null ? l : "user"),
      body: String((a = r.body) != null ? a : ""),
      status: r.status
    };
  });
}
async function G(e) {
  var r, l;
  const t = await fe(e);
  if (!t.length) return;
  const n = new Set(c.map((a) => a.id)), o = t.filter((a) => !n.has(a.id));
  o.length && (c.push(...o), b = (l = (r = c[c.length - 1]) == null ? void 0 : r.id) != null ? l : b, v.set("last_id", b), B(), C || (T += o.filter((a) => a.role !== "user").length), R());
}
async function ue(e) {
  if (!i || J || !e.trim()) return;
  J = !0;
  const t = e.trim(), n = q(), o = { id: n, role: "user", body: t, pending: !0 };
  c.push(o), v.set("last_id", n), B(), N();
  try {
    await ce(t, n);
  } catch {
    const r = c.findIndex((l) => l.id === n);
    r >= 0 && (c[r] = { ...c[r], status: "failed" });
  } finally {
    J = !1, B(), N();
  }
}
function ge() {
  !i || !W || j(`/plugins/chat/widget/typing?conversation=${i.conversationId}`, {
    method: "POST",
    body: "{}"
  }).catch(() => {
  });
}
function he() {
  !i || !C || j(`/plugins/chat/widget/read?conversation=${i.conversationId}`, {
    method: "POST",
    body: "{}"
  }).catch(() => {
  });
}
var ie;
let b = (ie = v.get("last_id")) != null ? ie : "";
async function we() {
  var t, n, o;
  if (!i) return;
  let e = 1e3;
  for (; F && i; ) {
    try {
      const r = await fetch(`${K}/events/session`, {
        headers: { Authorization: `Bearer ${i.token}` }
      });
      if (!r.ok || !r.body) throw new Error(`SSE ${r.status}`);
      W = !0, P(!1), e = 1e3;
      const l = r.body.getReader(), a = new TextDecoder();
      let m = "";
      for (; F; ) {
        const { done: L, value: f } = await l.read();
        if (L) break;
        m += a.decode(f, { stream: !0 });
        let u;
        for (; (u = m.indexOf(`

`)) >= 0; ) {
          const g = m.slice(0, u);
          if (m = m.slice(u + 2), g.startsWith("event:")) {
            const y = g.split(`
`), k = (n = (t = y.find((U) => U.startsWith("event:"))) == null ? void 0 : t.slice(6).trim()) != null ? n : "", z = (o = y.find((U) => U.startsWith("data:"))) == null ? void 0 : o.slice(5).trim();
            k && z && be(k, z);
          }
        }
      }
    } catch {
    }
    W = !1, P(!0), await new Promise((r) => setTimeout(r, e)), e = Math.min(e * 2, 3e4) + Math.floor(Math.random() * 500), i && await G(b).catch(() => {
    });
  }
}
function be(e, t) {
  var n, o, r, l, a, m, L, f;
  if (e === "chat.message.created")
    try {
      const u = JSON.parse(t), g = (o = (n = u == null ? void 0 : u.data) == null ? void 0 : n.data) != null ? o : u == null ? void 0 : u.data;
      if (!g || String(g.conversation_id) !== (i == null ? void 0 : i.conversationId)) return;
      const y = String((l = (r = g.message_id) != null ? r : g.id) != null ? l : q());
      if (y === b || c.some((k) => k.id === y)) return;
      if (g.role === "user") {
        const k = c.find((z) => z.pending);
        k && (k.id = y, k.pending = !1, b = y, v.set("last_id", y), B());
        return;
      }
      c.push({
        id: y,
        role: String((a = g.role) != null ? a : "assistant"),
        body: String((m = g.body) != null ? m : ""),
        status: g.status
      }), b = (f = (L = c[c.length - 1]) == null ? void 0 : L.id) != null ? f : y, v.set("last_id", b), B(), !C && g.role !== "user" && (T++, R()), N();
    } catch {
    }
}
function P(e) {
  I == null || I.classList.toggle("rw-conn", e);
}
let E, $, p, x, h, M, _, I, A;
function xe() {
  E = d("div", "rw-root"), E.dataset.position = S.position, document.body.appendChild(E);
  const e = document.createElement("style");
  e.textContent = se + `
.rw-root{--rw-brand:${S.color};}`, document.head.appendChild(e), $ = d("button", "rw-launcher"), $.setAttribute("aria-label", w("open")), $.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg>', _ = d("span", "rw-badge", "0"), $.appendChild(_), $.addEventListener("click", () => C ? V() : me()), E.appendChild($), p = d("div", "rw-panel"), p.setAttribute("role", "dialog"), p.setAttribute("aria-label", w("open"));
  const t = d("div", "rw-header"), n = d("div", "rw-title");
  if (n.textContent = S.avatar ? "" : w("open"), S.avatar) {
    const a = document.createElement("img");
    a.src = S.avatar, a.alt = "", a.className = "rw-avatar", t.appendChild(a);
  }
  I = d("span", "rw-conn"), I.textContent = "…";
  const o = d("button", "rw-close", "×");
  o.setAttribute("aria-label", "Close"), o.addEventListener("click", V), t.appendChild(n), t.appendChild(I), t.appendChild(o), p.appendChild(t), x = d("div", "rw-list"), p.appendChild(x), A = d("div", "rw-typing"), A.textContent = w("agentTyping"), A.hidden = !0, p.appendChild(A);
  const r = d("div", "rw-composer");
  h = document.createElement("input"), h.type = "text", h.placeholder = w("placeholder"), h.setAttribute("aria-label", w("placeholder"));
  let l;
  if (h.addEventListener("input", () => {
    window.clearTimeout(l), l = window.setTimeout(ge, 600);
  }), h.addEventListener("keydown", (a) => {
    a.key === "Enter" && !a.shiftKey && (a.preventDefault(), Q());
  }), M = d("button", "rw-send", w("send")), M.setAttribute("aria-label", w("send")), M.addEventListener("click", Q), r.appendChild(h), r.appendChild(M), p.appendChild(r), E.appendChild(p), "BroadcastChannel" in window) {
    const a = new BroadcastChannel("raisfast-chat");
    a.onmessage = (L) => {
      var u;
      const f = L.data;
      (f == null ? void 0 : f.type) === "unread" && (T = Number((u = f.count) != null ? u : 0)), (f == null ? void 0 : f.type) === "open" && (C = !!f.open, p.classList.toggle("rw-open", C)), R();
    };
    const m = () => a.postMessage({ type: "unread", count: T });
    window.addEventListener("unload", m);
  }
}
function Q() {
  const e = h.value;
  e.trim() && (h.value = "", ue(e));
}
function me() {
  C = !0, p.classList.add("rw-open"), T = 0, R(), G(b), he(), Y || (Y = !0, ye()), "BroadcastChannel" in window && new BroadcastChannel("raisfast-chat").postMessage({ type: "open", open: !0 }), setTimeout(() => h == null ? void 0 : h.focus(), 50);
}
function V() {
  C = !1, p.classList.remove("rw-open"), "BroadcastChannel" in window && new BroadcastChannel("raisfast-chat").postMessage({ type: "open", open: !1 });
}
function B() {
  x.replaceChildren();
  for (const e of c) {
    const t = d("div", `rw-msg ${e.role === "user" ? "rw-out" : "rw-in"}`), n = d("div", "rw-bubble");
    e.pending && n.classList.add("rw-pending"), e.status === "failed" && n.classList.add("rw-failed");
    const o = d("span", "rw-text");
    o.append(...le(e.body || "")), n.appendChild(o), t.appendChild(n), x.appendChild(t);
  }
  c.length || x.appendChild(d("div", "rw-empty", w("open"))), N();
}
function N() {
  p != null && p.classList.contains("rw-open") && (x == null || x.scrollTo({ top: x.scrollHeight }));
}
function R() {
  _ && (_.textContent = String(T), _.hidden = T === 0);
}
function ye() {
  var t;
  const e = (t = i == null ? void 0 : i.greeting) != null ? t : S.greeting;
  e && setTimeout(() => {
    if (!C) return;
    const n = d("div", "rw-greeting", e);
    E.appendChild(n), setTimeout(() => n.remove(), 6e3);
  }, 1500);
}
async function X() {
  var e;
  if (O) {
    xe();
    try {
      P(!0), await pe(), b = (e = v.get("last_id")) != null ? e : "", await G(b), F = !0, we(), P(!1);
    } catch {
      x.replaceChildren(d("div", "rw-empty", w("unavailable")));
      const t = d("button", "rw-send", w("retry"));
      t.addEventListener("click", () => location.reload()), x.appendChild(t);
    }
  }
}
document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", X) : X();
