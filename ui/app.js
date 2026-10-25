const invoke = window.__TAURI__?.core?.invoke;

const $ = (id) => document.getElementById(id);

const feed = $("feed");
const empty = $("empty");
const input = $("input");
const sendBtn = $("send");
const micBtn = $("mic");
const meta = $("meta");
const statusText = $("statusText");
const dot = $("dot");
const chips = $("chips");
const kvFiles = $("kvFiles");
const kvChunks = $("kvChunks");
const kvScope = $("kvScope");
const tabLabel = $("tabLabel");

const code = $("code");
const gutter = $("gutter");
const outBody = $("outBody");
const runMeta = $("runMeta");
const problems = $("problems");
const pmText = $("pmText");
const runBtn = $("runBtn");
const langRow = $("langRow");
const editorInfo = $("editorInfo");

const VIEW_TITLES = {
  interview: "模拟面试",
  exam: "模拟笔试",
  kb: "索引与来源",
  settings: "设置",
};

const TEMPLATES = {
  python:
    "# 输入从 stdin 读，输出打 stdout\nimport sys\n\ndata = sys.stdin.read().strip()\n\n",
  javascript:
    'const data = require("fs").readFileSync(0, "utf8").trim();\n\n',
  java:
    "import java.util.*;\n\npublic class Main {\n    public static void main(String[] args) {\n        Scanner sc = new Scanner(System.in);\n        \n    }\n}\n",
  cpp:
    "#include <bits/stdc++.h>\nusing namespace std;\n\nint main() {\n    \n    return 0;\n}\n",
};

const st = {
  view: "interview",
  runtimes: [],
  lang: "python",
  drafts: {},
  settings: null,
  outText: "",
  diagnostics: [],
  busy: false,
  answerSeq: 0,
};

const EXAMPLES = [
  "你们那个系统，置信度不够的时候怎么处理？",
  "为什么要做统一中间层，让 Skill 直接连流水线不行吗？",
  "RAG 上线之后模型老是编，你们怎么治的？",
];

// ---------------------------------------------------------------- 路由

function showView(name) {
  st.view = name;
  document.querySelectorAll(".view").forEach((v) => {
    v.classList.toggle("active", v.id === `view-${name}`);
  });
  document.querySelectorAll("[data-view]").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === name);
  });
  tabLabel.textContent = VIEW_TITLES[name] || name;
  if (name === "kb") loadKb();
}

document.querySelectorAll("[data-view]").forEach((b) => {
  b.addEventListener("click", () => showView(b.dataset.view));
});

// ---------------------------------------------------------------- 启动

chips.append(
  ...EXAMPLES.map((q) => {
    const b = document.createElement("button");
    b.className = "chip";
    b.type = "button";
    b.textContent = q;
    b.addEventListener("click", () => {
      showView("interview");
      input.value = q;
      resize();
      send();
    });
    return b;
  })
);

micBtn.disabled = true;

if (!invoke) {
  setStatus("bad", "未在 Sotto 应用内运行");
  meta.textContent = "没有 Tauri 注入";
} else {
  boot();
}

async function boot() {
  try {
    const s = await invoke("status");
    applyStatus(s);
    if (s.error) appendError(s.error);
  } catch (e) {
    setStatus("bad", "初始化失败");
    appendError(String(e));
  }

  try {
    st.runtimes = await invoke("runtimes");
  } catch (_) {
    st.runtimes = [];
  }
  renderLangs();

  try {
    st.settings = await invoke("get_settings");
    st.lang = st.settings.lang || "python";
    fillSettings(st.settings);
  } catch (_) {
    /* 没配置就先用默认 */
  }
  renderLangs();
  loadEditor(st.lang);
}

function applyStatus(s) {
  const files = s.files ?? 0;
  const chunks = s.chunks ?? 0;
  kvFiles.textContent = `${files} 篇`;
  kvChunks.textContent = `${chunks} 段`;
  const scope = Array.isArray(s.scope) ? s.scope : [];
  kvScope.textContent = scope.slice(1).join(" · ") || "—";
  kvScope.title = scope.join(" / ");

  if (s.ready) {
    setStatus("ok", "就绪");
    meta.textContent = `${chunks} 段 · 建库 ${(s.built_ms ?? 0).toFixed(0)}ms`;
  } else {
    setStatus("bad", "索引不可用");
    meta.textContent = s.error || "";
  }
}

function setStatus(kind, text) {
  dot.className = "dot" + (kind === "ok" ? " ok" : kind === "bad" ? " bad" : "");
  statusText.textContent = text;
}

// ---------------------------------------------------------------- 模拟面试

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
  input.style.height = Math.min(input.scrollHeight, 104) + "px";
}

async function send() {
  const q = input.value.trim();
  if (!q || st.busy || !invoke) return;

  st.busy = true;
  sendBtn.disabled = true;
  input.value = "";
  resize();

  if (empty && empty.parentNode) empty.remove();
  appendAsk(q);
  scrollToEnd();

  const reqId = ++st.answerSeq;
  const card = createAnswerCard();
  feed.appendChild(card);
  scrollToEnd();

  // 边收边渲染：模型每个 delta 都推到这里
  let unlisten = null;
  let streamed = "";
  try {
    if (window.__TAURI__?.event?.listen) {
      unlisten = await window.__TAURI__.event.listen("answer-delta", (e) => {
        if (!e.payload || e.payload.id !== reqId) return;
        streamed += e.payload.delta;
        renderStreaming(card, streamed);
        scrollToEnd();
      });
    }

    const res = await invoke("answer", { reqId, question: q, k: st.settings?.top_k ?? 3 });
    renderAnswer(card, res);
    meta.textContent = `首字 ${res.ttft_ms.toFixed(0)} ms · 共 ${res.total_ms.toFixed(0)} ms · ${res.chars} 字`;
  } catch (e) {
    // 模型没配好或者挂了：退回「只给材料」，至少别让这一问白问
    card.remove();
    appendNote(streamed ? "生成中断，以下是命中的材料" : String(e));
    try {
      const res = await invoke("search", { query: q, k: st.settings?.top_k ?? 3 });
      appendResults(res);
      meta.textContent = `${res.hits.length} 条材料 · 未生成回答`;
    } catch (_) {
      /* 检索也挂了就算了 */
    }
  } finally {
    if (unlisten) unlisten();
    st.busy = false;
    sendBtn.disabled = false;
    input.focus();
    scrollToEnd();
  }
}

function createAnswerCard() {
  const card = document.createElement("article");
  card.className = "answer";

  const head = document.createElement("div");
  head.className = "answer-head";
  const badge = document.createElement("span");
  badge.className = "answer-badge";
  badge.textContent = "Sotto";
  const metaEl = document.createElement("span");
  metaEl.className = "answer-meta";
  metaEl.textContent = "正在想…";
  head.append(badge, metaEl);

  const lead = document.createElement("p");
  lead.className = "answer-lead";
  lead.hidden = true;

  const body = document.createElement("p");
  body.className = "answer-body streaming";

  const sources = document.createElement("div");
  sources.className = "answer-sources";
  sources.hidden = true;

  card.append(head, lead, body, sources);
  card._parts = { metaEl, lead, body, sources };
  return card;
}

function renderStreaming(card, text) {
  const { metaEl, body } = card._parts;
  body.textContent = text;
  metaEl.textContent = "生成中…";
}

function renderAnswer(card, res) {
  const { metaEl, lead, body, sources } = card._parts;
  lead.textContent = res.lead || res.answer;
  lead.hidden = false;
  body.classList.remove("streaming");
  // 首句如果已经在 lead 里，body 就只放剩下的
  const rest = res.answer.startsWith(res.lead)
    ? res.answer.slice(res.lead.length).trim()
    : res.answer;
  body.textContent = rest;
  body.hidden = !rest;

  metaEl.textContent = `首字 ${res.ttft_ms.toFixed(0)} ms · 共 ${res.total_ms.toFixed(0)} ms · ${res.chars} 字`;

  sources.innerHTML = "";
  (res.sources || []).forEach((s) => {
    const chip = document.createElement("span");
    chip.className = "src-chip";
    chip.textContent = s.heading ? `${s.source} · ${s.heading}` : s.source;
    chip.title = chip.textContent;
    sources.appendChild(chip);
  });
  sources.hidden = !(res.sources || []).length;
}

function appendNote(text) {
  const note = document.createElement("p");
  note.className = "note";
  note.textContent = text;
  feed.appendChild(note);
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
  note.textContent = res.degraded
    ? "命中很弱 · 材料里可能没有直接对应，接模型后会降级为通用回答"
    : `命中 ${res.hits.length} 条材料 · 未接模型，以下是原文摘录`;
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
  if (feed) feed.appendChild(box);
  scrollToEnd();
}

function scrollToEnd() {
  requestAnimationFrame(() => {
    if (feed) feed.scrollTop = feed.scrollHeight;
  });
}

// ---------------------------------------------------------------- 模拟笔试

function renderLangs() {
  langRow.innerHTML = "";
  const list = st.runtimes.length
    ? st.runtimes
    : [
        { id: "python", label: "Python 3", available: true, version: "" },
        { id: "javascript", label: "JavaScript", available: true, version: "" },
        { id: "java", label: "Java", available: false, version: "" },
        { id: "cpp", label: "C++17", available: false, version: "" },
      ];

  list.forEach((r) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "lang-chip" + (r.id === st.lang ? " active" : "");
    b.textContent = r.label;
    if (!r.available) {
      b.classList.add("off");
      b.title = "本机没找到这个运行时";
      b.disabled = true;
    } else if (r.version) {
      b.title = r.version;
    }
    b.addEventListener("click", () => loadEditor(r.id));
    langRow.appendChild(b);
  });

  const cur = list.find((r) => r.id === st.lang);
  editorInfo.textContent = cur ? `${cur.label}${cur.version ? " · " + cur.version : ""}` : st.lang;
}

function loadEditor(lang) {
  if (st.drafts[st.lang] !== undefined) st.drafts[st.lang] = code.value;
  st.lang = lang;
  code.value = st.drafts[lang] ?? TEMPLATES[lang] ?? "";
  renderLangs();
  renderGutter();
  markError(null);
  syncEditorHeight();
  clearCheck();
  st.outText = "";
  renderOut();
  runMeta.textContent = "";
  runMeta.className = "out-meta";
}

function renderGutter() {
  const n = code.value.split("\n").length;
  const frag = document.createDocumentFragment();
  for (let i = 1; i <= n; i++) {
    const d = document.createElement("div");
    d.className = "ln";
    d.textContent = String(i);
    frag.appendChild(d);
  }
  gutter.innerHTML = "";
  gutter.appendChild(frag);
}

function markLines(lines) {
  gutter.querySelectorAll(".ln.err").forEach((d) => d.classList.remove("err"));
  (lines || []).forEach((n) => {
    // EOF 类错误报在「最后一行之后」，夹到末行上才有东西可标
    const idx = Math.min(Math.max(n, 1), gutter.children.length) - 1;
    const el = gutter.children[idx];
    if (el) el.classList.add("err");
  });
  code.classList.toggle("has-err", (lines || []).length > 0);
}

function markError(line) {
  markLines(line ? [line] : []);
}

code.addEventListener("input", () => {
  renderGutter();
  markError(null);
  syncEditorHeight();
  scheduleCheck();
});
// 粘贴/撤销/输入法这类路径不一定都走 input，兜一层
code.addEventListener("keyup", () => {
  renderGutter();
  syncEditorHeight();
});
code.addEventListener("change", () => {
  renderGutter();
  syncEditorHeight();
});
function syncEditorHeight() {
  if (!code) return;
  const pane = $("editorBox")?.clientHeight ?? 0;
  code.style.height = "auto";
  code.style.height = Math.max(code.scrollHeight + 2, pane) + "px";
}

window.addEventListener("resize", syncEditorHeight);
code.addEventListener("keydown", (e) => {
  if (e.key === "Tab") {
    e.preventDefault();
    const start = code.selectionStart;
    const end = code.selectionEnd;
    code.value = code.value.slice(0, start) + "    " + code.value.slice(end);
    code.selectionStart = code.selectionEnd = start + 4;
    renderGutter();
  }
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    runCode();
  }
});

document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "r" && st.view === "exam") {
    e.preventDefault();
    runCode();
  }
});

runBtn.addEventListener("click", runCode);

// ---------------------------------------------------------------- 静态检查
// 输入停顿后让编译器做语法检查，不用先跑一遍。
let checkTimer = null;
let checkToken = 0;

function scheduleCheck() {
  if (!invoke || st.view !== "exam") return;
  if (checkTimer) clearTimeout(checkTimer);
  // Java 编译一次要一秒，给它更长的静默窗口
  checkTimer = setTimeout(runCheck, st.lang === "java" ? 900 : 450);
}

async function runCheck() {
  const token = ++checkToken;
  if (!code.value.trim()) {
    showProblems([]);
    return;
  }
  try {
    const list = await invoke("check", { lang: st.lang, code: code.value });
    if (token !== checkToken) return; // 已经有更新的一次检查在路上了
    showProblems(list || []);
  } catch (_) {
    // 检查失败不该打断写代码，静默就好
  }
}

function clearCheck() {
  checkToken++;
  if (checkTimer) clearTimeout(checkTimer);
  showProblems([]);
}

function showProblems(list) {
  st.diagnostics = list;
  markLines(list.map((d) => d.line));
  if (!list.length) {
    problems.hidden = true;
    return;
  }
  const first = list[0];
  pmText.textContent =
    list.length === 1
      ? `第 ${first.line} 行 · ${first.message}`
      : `${list.length} 个问题 · 第 ${first.line} 行 ${first.message}`;
  problems.hidden = false;
}

problems.addEventListener("click", () => {
  if (st.diagnostics.length) gotoLine(st.diagnostics[0].line);
});

/// 把光标放到第 n 行，并滚到可见范围中间。
function gotoLine(n) {
  const lines = code.value.split("\n");
  const target = Math.min(Math.max(n, 1), lines.length);
  let offset = 0;
  for (let i = 0; i < target - 1; i++) offset += lines[i].length + 1;
  code.focus();
  code.setSelectionRange(offset, offset + (lines[target - 1] || "").length);

  const pane = $("editorBox");
  const lineTop = 12 + (target - 1) * 20;
  if (lineTop < pane.scrollTop || lineTop + 20 > pane.scrollTop + pane.clientHeight) {
    pane.scrollTop = Math.max(0, lineTop - pane.clientHeight / 2);
  }
}

async function runCode() {
  if (!invoke) return;
  clearCheck();
  runBtn.disabled = true;
  runMeta.textContent = "运行中…";
  runMeta.className = "out-meta";

  try {
    const r = await invoke("run_code", {
      lang: st.lang,
      code: code.value,
      stdin: $("stdin").value,
    });

    st.outText = mergeOutput(r);
    markError(r.error_line);
    runMeta.textContent = describe(r);
    runMeta.className = "out-meta " + (r.status === "ok" ? "ok" : "bad");
  } catch (e) {
    st.outText = String(e);
    markError(null);
    runMeta.textContent = "执行失败";
    runMeta.className = "out-meta bad";
  } finally {
    runBtn.disabled = false;
    renderOut();
  }
}

function describe(r) {
  const ms = `${r.duration_ms.toFixed(0)} ms`;
  if (r.status === "timeout") return `超时（3s 上限）· 检查死循环`;
  if (r.status === "compile_error")
    return `编译错误${r.error_line ? " · 第 " + r.error_line + " 行" : ""} · ${ms}`;
  if (r.status === "runtime_error")
    return `运行错误${r.error_line ? " · 第 " + r.error_line + " 行" : ""} · ${ms}`;
  return `完成 · ${ms}`;
}

/// 输出和报错合并成一块：有正常输出就先给输出，报错接在后面。
/// 编译不过时 stdout 是空的，于是这里就只剩报错。
function mergeOutput(r) {
  const out = (r.stdout || "").trimEnd();
  const err = (r.stderr || "").trimEnd();
  if (out && err) return out + "\n\n--- 错误输出 ---\n" + err;
  return out || err;
}

function renderOut() {
  const text = st.outText || "";
  outBody.textContent = text || "（没有输出）";
  outBody.classList.toggle("dim", !text);
}

// ---------------------------------------------------------------- 知识库

$("reindexBtn").addEventListener("click", async () => {
  if (!invoke) return;
  const btn = $("reindexBtn");
  btn.disabled = true;
  btn.textContent = "重建中…";
  try {
    const kb = await invoke("reindex");
    renderKb(kb);
    const s = await invoke("status");
    applyStatus(s);
  } catch (e) {
    $("kbSub").textContent = String(e);
  } finally {
    btn.disabled = false;
    btn.textContent = "重建索引";
  }
});

async function loadKb() {
  if (!invoke) return;
  try {
    renderKb(await invoke("kb_report"));
  } catch (e) {
    $("kbSub").textContent = String(e);
  }
}

function renderKb(kb) {
  $("kbSub").textContent = `${kb.root} · ${kb.files.length} 篇 / ${kb.chunks} 段 / ${(
    kb.total_chars / 10000
  ).toFixed(1)} 万字 · 建库 ${kb.built_ms.toFixed(0)}ms`;

  const rows = $("fileRows");
  rows.innerHTML = "";
  kb.files.forEach((f) => {
    const tr = document.createElement("tr");
    const name = document.createElement("td");
    name.textContent = f.rel;
    name.title = f.rel;
    const c = document.createElement("td");
    c.className = "num";
    c.textContent = String(f.chunks);
    const w = document.createElement("td");
    w.className = "num";
    w.textContent = String(f.chars);
    const b = document.createElement("td");
    b.className = "num";
    b.textContent = `${(f.bytes / 1024).toFixed(1)}K`;
    tr.append(name, c, w, b);
    rows.appendChild(tr);
  });

  $("skipCount").textContent = kb.skipped.length ? `（${kb.skipped.length}）` : "";
  const list = $("skipList");
  list.innerHTML = "";
  if (!kb.skipped.length) {
    const p = document.createElement("p");
    p.className = "hint";
    p.textContent = "没有跳过任何文件。";
    list.appendChild(p);
    return;
  }
  kb.skipped.forEach((s) => {
    const row = document.createElement("div");
    row.className = "skip-row";
    const r = document.createElement("span");
    r.className = "skip-rel";
    r.textContent = s.rel;
    const reason = document.createElement("span");
    reason.className = "skip-reason";
    reason.textContent = s.reason;
    row.append(r, reason);
    list.appendChild(row);
  });
}

// ---------------------------------------------------------------- 设置

function fillSettings(cfg) {
  $("setKey").value = cfg.api_key ?? "";
  $("setModel").value = cfg.model ?? "";
  $("setBase").value = cfg.base_url ?? "";
  $("setTemp").value = cfg.temperature ?? 0.4;
  $("setRoot").value = cfg.root ?? "";
  $("setIncludes").value = (cfg.includes ?? []).join(", ");
  $("setExcludes").value = (cfg.excludes ?? []).join(", ");
  $("setMaxKb").value = cfg.max_file_kb ?? 512;
  $("setTopK").value = cfg.top_k ?? 3;
}

function readSettings() {
  const split = (v) =>
    v
      .split(/[,，]/)
      .map((s) => s.trim())
      .filter(Boolean);
  return {
    api_key: $("setKey").value.trim(),
    model: $("setModel").value.trim(),
    base_url: $("setBase").value.trim(),
    temperature: Number($("setTemp").value) || 0.4,
    root: $("setRoot").value.trim(),
    includes: split($("setIncludes").value),
    excludes: split($("setExcludes").value),
    max_file_kb: Number($("setMaxKb").value) || 512,
    top_k: Math.min(10, Math.max(1, Number($("setTopK").value) || 3)),
    lang: st.lang,
  };
}

$("saveBtn").addEventListener("click", async () => {
  if (!invoke) return;
  const msg = $("saveMsg");
  msg.textContent = "保存中…";
  msg.className = "save-msg";
  try {
    const s = await invoke("save_settings", { cfg: readSettings() });
    st.settings = await invoke("get_settings");
    applyStatus(s);
    msg.textContent = "已保存，索引已重建";
    msg.className = "save-msg ok";
    loadKb();
  } catch (e) {
    msg.textContent = String(e);
    msg.className = "save-msg bad";
  }
});

$("probeBtn").addEventListener("click", async () => {
  if (!invoke) return;
  const msg = $("probeMsg");
  msg.textContent = "连接中…";
  msg.className = "save-msg";
  // 先把当前表单存下去，否则探测的还是旧 key
  try {
    await invoke("save_settings", { cfg: readSettings() });
  } catch (e) {
    msg.textContent = String(e);
    msg.className = "save-msg bad";
    return;
  }
  try {
    const r = await invoke("llm_probe");
    msg.textContent = `通了 · 首字 ${r.ttft_ms.toFixed(0)} ms · 返回「${(r.text || "").slice(0, 12)}」`;
    msg.className = "save-msg ok";
  } catch (e) {
    msg.textContent = String(e);
    msg.className = "save-msg bad";
  }
});

$("resetBtn").addEventListener("click", () => {
  fillSettings({
    root: st.settings?.root ?? $("setRoot").value,
    includes: ["项目经历", "面试准备", "学习"],
    excludes: [".qoder", ".obsidian", ".git", ".trash", "node_modules", "target", ".venv"],
    max_file_kb: 512,
    top_k: 3,
  });
  $("saveMsg").textContent = "已填回默认值，点保存才生效";
  $("saveMsg").className = "save-msg";
});
