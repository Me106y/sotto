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
  out: { stdout: "", stderr: "" },
  outTab: "stdout",
  busy: false,
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
  const thinking = appendThinking();
  scrollToEnd();

  try {
    const res = await invoke("search", { query: q, k: st.settings?.top_k ?? 3 });
    thinking.remove();
    appendResults(res);
    meta.textContent = `${res.hits.length} 条命中 · ${res.took_ms.toFixed(1)} ms`;
  } catch (e) {
    thinking.remove();
    appendError(String(e));
  } finally {
    st.busy = false;
    sendBtn.disabled = false;
    input.focus();
    scrollToEnd();
  }
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
  st.out = { stdout: "", stderr: "" };
  st.outTab = "stdout";
  setOutTab("stdout");
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

function markError(line) {
  gutter.querySelectorAll(".ln.err").forEach((d) => d.classList.remove("err"));
  code.classList.remove("has-err");
  if (line && gutter.children[line - 1]) {
    gutter.children[line - 1].classList.add("err");
    code.classList.add("has-err");
  }
}

code.addEventListener("input", () => {
  renderGutter();
  markError(null);
});
code.addEventListener("scroll", () => {
  gutter.scrollTop = code.scrollTop;
});
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

async function runCode() {
  if (!invoke) return;
  runBtn.disabled = true;
  runMeta.textContent = "运行中…";
  runMeta.className = "out-meta";

  try {
    const r = await invoke("run_code", {
      lang: st.lang,
      code: code.value,
      stdin: $("stdin").value,
    });

    st.out = { stdout: r.stdout || "", stderr: r.stderr || "" };
    markError(r.error_line);
    setOutTab(r.stderr ? "stderr" : "stdout");

    const expected = $("expected").value;
    if (expected.trim() && r.status === "ok") {
      const pass = (r.stdout || "").trim() === expected.trim();
      runMeta.textContent = `${pass ? "✓ 通过" : "✗ 输出不匹配"} · ${r.duration_ms.toFixed(0)} ms`;
      runMeta.className = "out-meta " + (pass ? "ok" : "bad");
    } else {
      runMeta.textContent = describe(r);
      runMeta.className = "out-meta " + (r.status === "ok" ? "ok" : "bad");
    }
  } catch (e) {
    st.out = { stdout: "", stderr: String(e) };
    setOutTab("stderr");
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

document.querySelectorAll(".out-tab").forEach((b) => {
  b.addEventListener("click", () => setOutTab(b.dataset.out));
});

function setOutTab(name) {
  st.outTab = name;
  document.querySelectorAll(".out-tab").forEach((b) => {
    b.classList.toggle("active", b.dataset.out === name);
  });
  renderOut();
}

function renderOut() {
  const text = st.out[st.outTab] || "";
  outBody.textContent = text || (st.outTab === "stdout" ? "（没有输出）" : "（没有错误）");
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
