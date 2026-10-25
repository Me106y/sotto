const invoke = window.__TAURI__?.core?.invoke;

const feed = document.getElementById("feed");
const empty = document.getElementById("empty");
const input = document.getElementById("input");
const sendBtn = document.getElementById("send");
const micBtn = document.getElementById("mic");
const meta = document.getElementById("meta");
const statusText = document.getElementById("statusText");
const statusDot = document.querySelector("#status .dot");
const chips = document.getElementById("chips");

const EXAMPLES = [
  "你们那个系统，置信度不够的时候怎么处理？",
  "为什么要做统一中间层，让 Skill 直接连流水线不行吗？",
  "RAG 上线之后模型老是编，你们怎么治的？",
];

let busy = false;

// ---------------------------------------------------------------- 初始化

EXAMPLES.forEach((q) => {
  const b = document.createElement("button");
  b.className = "chip";
  b.type = "button";
  b.textContent = q;
  b.addEventListener("click", () => {
    input.value = q;
    resize();
    send();
  });
  chips.appendChild(b);
});

micBtn.disabled = true;

if (!invoke) {
  setStatus("bad", "未在 Sotto 应用内运行");
  meta.textContent = "请在 Sotto 桌面应用里打开这个界面（当前没有 Tauri 注入）。";
} else {
  invoke("status")
    .then((s) => {
      if (!s.ready) {
        setStatus("bad", "索引不可用");
        appendError(s.error || "索引未就绪");
        return;
      }
      setStatus("ok", `${s.files} 篇 · ${s.chunks} 段`);
      meta.textContent = `索引 ${s.chunks} 段材料，来源：${shortScope(s.scope)}`;
    })
    .catch((e) => {
      setStatus("bad", "初始化失败");
      appendError(String(e));
    });
}

function setStatus(kind, text) {
  statusDot.className = "dot" + (kind === "ok" ? " ok" : kind === "bad" ? " bad" : "");
  statusText.textContent = text;
}

function shortScope(scope) {
  if (!Array.isArray(scope) || scope.length === 0) return "—";
  const dirs = scope.slice(1);
  return dirs.length ? dirs.join(" / ") : scope[0];
}

// ---------------------------------------------------------------- 交互

input.addEventListener("input", resize);
input.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  }
});
sendBtn.addEventListener("click", () => send());

function resize() {
  input.style.height = "auto";
  input.style.height = Math.min(input.scrollHeight, 108) + "px";
}

async function send() {
  const q = input.value.trim();
  if (!q || busy || !invoke) return;

  busy = true;
  sendBtn.disabled = true;
  input.value = "";
  resize();

  hideEmpty();
  appendAsk(q);
  const thinking = appendThinking();
  scrollToEnd();

  try {
    const res = await invoke("search", { query: q, k: 3 });
    thinking.remove();
    appendResults(res);
    meta.textContent = `${res.hits.length} 条命中 · ${res.took_ms.toFixed(1)} ms · 索引 ${res.chunks} 段`;
  } catch (e) {
    thinking.remove();
    appendError(String(e));
  } finally {
    busy = false;
    sendBtn.disabled = false;
    input.focus();
    scrollToEnd();
  }
}

function hideEmpty() {
  if (empty && empty.parentNode) empty.remove();
}

function appendAsk(text) {
  const row = document.createElement("div");
  row.className = "ask";
  const p = document.createElement("p");
  p.textContent = text;
  row.appendChild(p);
  feed.appendChild(row);
}

function appendThinking() {
  const box = document.createElement("div");
  box.className = "thinking";
  box.innerHTML = "<i></i><i></i><i></i>";
  feed.appendChild(box);
  return box;
}

function appendResults(res) {
  if (!res.hits.length) {
    const note = document.createElement("p");
    note.className = "note";
    note.textContent = "没命中。可能这条没写过，或者作用域没圈到。";
    feed.appendChild(note);
    return;
  }

  const note = document.createElement("p");
  note.className = "note";
  note.textContent = `命中 ${res.hits.length} 条材料（M0 只做检索，未接模型）`;
  feed.appendChild(note);

  res.hits.forEach((h) => feed.appendChild(renderCard(h)));
}

function renderCard(h) {
  const card = document.createElement("article");
  card.className = "card";

  const head = document.createElement("div");
  head.className = "card-head";

  const rank = document.createElement("span");
  rank.className = "rank";
  rank.textContent = String(h.rank);

  const src = document.createElement("span");
  src.className = "src";
  src.textContent = h.heading ? `${h.source} · ${h.heading}` : h.source;
  src.title = `${h.source} · ${h.heading}`;

  const score = document.createElement("span");
  score.className = "score";
  score.textContent = h.score.toFixed(2);

  head.append(rank, src, score);

  const lead = document.createElement("p");
  lead.className = "lead";
  lead.textContent = h.lead;

  const body = document.createElement("p");
  body.className = "body";
  body.textContent = h.body;

  card.append(head, lead, body);
  return card;
}

function appendError(text) {
  const box = document.createElement("div");
  box.className = "error";
  box.textContent = text;
  feed.appendChild(box);
  scrollToEnd();
}

function scrollToEnd() {
  requestAnimationFrame(() => {
    feed.scrollTop = feed.scrollHeight;
  });
}
