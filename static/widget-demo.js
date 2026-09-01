// RaisFast Chat Widget 集成测试页 — 页面逻辑（独立文件以符合 CSP script-src 'self'）。
// 动态注入 widget.js（同源，CSP 允许），并把 data-* 传给 document.currentScript。

(function () {
  "use strict";
  const $ = (id) => document.getElementById(id);
  const statusEl = $("status");

  function loadWidget() {
    const channel = $("channel").value.trim();
    if (!channel) { setStatus("请填写渠道 key", true); return; }
    const color = $("color").value.trim() || "#4f46e5";
    const position = $("position").value;
    const locale = $("locale").value;

    // 卸载旧的 widget 根节点（widget 在 body 挂 #raf-widget-root）。
    document.getElementById("raf-widget-root")?.remove();

    const script = document.createElement("script");
    script.src = "./widget.js"; // static/widget.js（= 后端 /static/widget.js）
    script.setAttribute("data-channel", channel);
    script.setAttribute("data-color", color);
    script.setAttribute("data-position", position);
    script.setAttribute("data-locale", locale);
    script.defer = true;
    script.onload = () => setStatus("Widget 已加载：渠道 " + channel);
    script.onerror = () => setStatus("加载 ./widget.js 失败：请确认后端已启动且该文件存在", true);
    document.body.appendChild(script);
  }

  function resetSession() {
    const channel = $("channel").value.trim() || "chat-widget-demo";
    try {
      // 清掉该渠道的 widget 会话缓存（匿名身份 / token / 消息游标）。
      Object.keys(localStorage)
        .filter((k) => k.indexOf("raf.widget." + channel + ".") === 0)
        .forEach((k) => localStorage.removeItem(k));
    } catch (e) { /* localStorage 不可用时忽略 */ }
    document.getElementById("raf-widget-root")?.remove();
    setStatus("已重置渠道 " + channel + " 的本地会话，可重新加载");
  }

  function setStatus(msg, isError) {
    statusEl.textContent = msg;
    statusEl.style.color = isError ? "#dc2626" : "";
  }

  $("load").addEventListener("click", loadWidget);
  $("reset").addEventListener("click", resetSession);
  $("channel").addEventListener("keydown", (e) => { if (e.key === "Enter") loadWidget(); });
})();
