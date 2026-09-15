import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import "./styles.css";

type DiagnosticSummary = {
  app_name: string;
  root: string;
  database: string;
  schema_version: number;
  index_status: string;
  notes: string[];
};

type InputQuality = {
  can_recommend: boolean;
  low_info: boolean;
  filled_fields: number;
  message: string;
};

type IndexHealth = {
  status: string;
  detail: string;
  dirty_chunks: number;
  index_bytes: number;
};

type Hint = {
  level: "Normal" | "Important" | "Danger";
  text: string;
};

type AttachmentStatus = {
  storedPath: string;
  textLength: number;
  warning: string;
  privacyNote: string;
};

type JournalRecord = {
  id: number;
  name: string;
  issn: string;
  eissn: string;
  source_db: string;
  jif_quartile: string;
  cas_quartile: string;
  cas_zone: string;
  impact_factor: number | null;
  wos_articles: number | null;
  subjects: string;
  publisher: string;
  ccf: string;
  ei: string;
  cssci: string;
  cscd: string;
  pku_core: string;
  ami_level: string;
  top_flag: string;
  oa_flag: string;
  oa_detail: string;
  website: string;
  scope_text: string;
  jcr_rank_detail: string;
  difficulty: string;
  recommendation_preference: string;
  apc: string;
  review_cycle: string;
  tags: string;
  summary: string;
  experience: string;
  rejection_reason: string;
  suitable_topics: string;
  avoid_reason: string;
  article_topics: string;
  submission_guidelines: string;
  grade: string;
};

type SubmissionRecord = {
  id: number;
  title: string;
  authors: string;
  corresponding: string;
  journal_name: string;
  submit_date: string;
  status: string;
  result_date: string;
  apc_paid: string;
  website: string;
  notes: string;
  created_at: string;
};

const pages = ["期刊雷达", "AI 推荐投稿", "我的收藏", "投稿记录", "投稿模板", "资源导入", "设置与诊断"];
let activePage = pages[0];
let diagnostics: DiagnosticSummary | null = null;
let indexHealth: IndexHealth | null = null;
let hints: Hint[] = [];
let inputQuality: InputQuality | null = null;
let journals: JournalRecord[] = [];
let journalLoading = false;
let journalError = "";
let journalTotal = 0;
let submissions: SubmissionRecord[] = [];
let submissionError = "";
let attachmentStatus: AttachmentStatus | null = null;
let selectedStrategy = "均衡推荐";
let quickHealthHtml = "";
let lastRecommendationItems: any[] = [];
let recommendationInputKey = "";
let recommendationRefreshRound = 0;
let seenRecommendationIds: number[] = [];
let seenRecommendationNames: string[] = [];
let webRefreshRound = 0;
let seenWebTitles: string[] = [];
let editingJournal: JournalRecord | null = null;
let editingSubmission: SubmissionRecord | null | undefined = undefined;
let journalPage = 0;
const journalPageSize = 50;
let targetJifSelection: string[] = [];
let subjectSelection: string[] = [];
let attachmentFile: File | null = null;
let resourceFile: File | null = null;
let templateFile: File | null = null;
let templateItems: any[] = [];
let templateTotal = 0;
let templatePage = 0;
const templatePageSize = 12;
let busyCount = 0;
let busyMessage = "正在处理中，请稍候...";

const busyLabels: Record<string, string> = {
  ai_recommend: "正在生成 AI 推荐，请稍候...",
  local_fallback_recommend: "正在检索本地期刊库，请稍候...",
  web_search_sources: "正在联网补充真实期刊记录，请稍候...",
  enrich_journals: "正在抓取期刊真实数据，请稍候...",
  upload_attachment: "正在保存并提取附件正文，请稍候...",
  upload_resource: "正在导入资源文件，请稍候...",
  list_journals: "正在读取期刊库，请稍候...",
  count_journals: "正在统计匹配期刊，请稍候...",
  rebuild_rag_index: "正在重建 RAG 索引，请稍候...",
  import_legacy_data: "正在迁移旧版数据，请稍候...",
};

function updateBusyOverlay() {
  const overlay = document.querySelector<HTMLElement>("#busyOverlay");
  const message = document.querySelector<HTMLElement>("#busyMessage");
  if (message) message.textContent = busyMessage;
  if (overlay) overlay.classList.toggle("visible", busyCount > 0);
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  busyCount += 1;
  busyMessage = busyLabels[command] ?? "正在处理，请稍候...";
  updateBusyOverlay();
  try {
    return await tauriInvoke<T>(command, args);
  } finally {
    busyCount = Math.max(0, busyCount - 1);
    updateBusyOverlay();
  }
}

const submissionState = {
  keyword: "",
  status: "",
};

const libraryState = {
  keyword: "",
  difficulty: "",
};

const radarState = {
  keyword: "",
  source: "",
  jif: [] as string[],
  cas: [] as string[],
  ccf: false,
  ei: false,
};

const state = {
  title: "",
  abstractText: "",
  keywords: "",
  fullText: "",
};

const aiState = {
  serviceKey: "openrouter",
  baseUrl: "https://openrouter.ai/api/v1",
  model: "openrouter/free",
  apiKey: "",
};

const privacyState = {
  localOnly: false,
  disableWebSearch: false,
  doNotSendFullText: false,
  aiTitleAbstractOnly: false,
};

const subjectOptions = [
  "计算机科学",
  "智能交通/交通工程",
  "管理科学",
  "马克思主义",
  "教育学",
  "医学",
  "历史学",
  "文学",
];

function hintClass(level: Hint["level"]): string {
  if (level === "Danger") return "hint danger";
  if (level === "Important") return "hint warning";
  return "hint normal";
}

function renderHint(text: string, level: Hint["level"] = "Normal") {
  return `<div class="${hintClass(level)}">${text}</div>`;
}

function renderShell() {
  const app = document.querySelector<HTMLDivElement>("#app")!;
  app.innerHTML = `
    <main class="layout">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-mark">投</div>
          <div>
            <h1>投刊机器人</h1>
            <p>本地优先 · RAG 推荐 · 投稿经验库</p>
          </div>
        </div>
        <nav>
          ${pages.map((page) => `<button class="nav-item ${page === activePage ? "active" : ""}" data-page="${page}">${page}</button>`).join("")}
        </nav>
        <section class="status-panel">
          <span class="status-dot"></span>
          <div>
            <strong>${diagnostics ? "本地数据已就绪" : "正在初始化"}</strong>
            <small>${diagnostics?.root ?? "D:\\投刊机器人"}</small>
          </div>
        </section>
      </aside>
      <section class="content">
        <header class="topbar">
          <div>
            <h2>${activePage}</h2>
          </div>
          <button class="primary" id="refreshDiagnostics">刷新</button>
        </header>
        ${renderPage()}
      </section>
    </main>
    ${renderActiveModal()}
    <div class="busy-overlay ${busyCount > 0 ? "visible" : ""}" id="busyOverlay" aria-live="polite">
      <div class="busy-card"><span class="spinner"></span><strong id="busyMessage">${escapeHtml(busyMessage)}</strong><small>操作完成前请不要重复点击。</small></div>
    </div>
  `;

  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
    button.addEventListener("click", () => {
      activePage = button.dataset.page || pages[0];
      renderShell();
    });
  });
  document.querySelector<HTMLButtonElement>("#refreshDiagnostics")?.addEventListener("click", initialize);
  bindPageEvents();
  bindModalEvents();
  bindRecommendationActionEvents();
}

function renderPage() {
  if (activePage === "期刊雷达") return renderRadarPage();
  if (activePage === "AI 推荐投稿") return renderAiPage();
  if (activePage === "我的收藏") return renderLibraryPage();
  if (activePage === "投稿记录") return renderSubmissionsPage();
  if (activePage === "投稿模板") return renderTemplatesPage();
  if (activePage === "资源导入") return renderImportPage();
  return renderSettingsPage();
}

function renderRadarPage() {
  return `
    <section class="panel-grid">
      <article class="panel wide">
        <h3>期刊雷达</h3>
        ${renderHint("计算机、马克思主义、全部学科筛选必须完整保留；来源、分区可组合筛选。", "Important")}
        ${renderHint("JIF 分区里的 Q1 只是分位标签，系统会尽量再抓官网上的 Rank / Top% / Percentile；抓不到时只保留已核验的分区信息，不会乱填。", "Important")}
        <div class="radar-filters">
          <input id="radarKeyword" value="${escapeHtml(radarState.keyword)}" placeholder="搜索期刊名、学科或出版社" />
          <select id="radarSource">
            <option value="">全部来源库</option>
            ${["SCI", "SCIE", "SSCI", "CSSCI", "CSCD", "CCF"].map((value) => `<option value="${value}" ${radarState.source === value ? "selected" : ""}>${value}</option>`).join("")}
          </select>
          <label class="check"><input type="checkbox" id="radarCcf" ${radarState.ccf ? "checked" : ""}/> CCF</label>
          <label class="check"><input type="checkbox" id="radarEi" ${radarState.ei ? "checked" : ""}/> EI</label>
          <button class="primary" id="searchJournals">查询</button>
        </div>
        <div class="filter-group"><span>JIF 分区</span>${["Q1", "Q2", "Q3", "Q4"].map((value) => `<label class="check"><input type="checkbox" data-radar-jif value="${value}" ${radarState.jif.includes(value) ? "checked" : ""}/> ${value}</label>`).join("")}</div>
        <div class="filter-group"><span>中科院分区</span>${["1", "2", "3", "4"].map((value) => `<label class="check"><input type="checkbox" data-radar-cas value="${value}" ${radarState.cas.includes(value) ? "checked" : ""}/> ${value} 区</label>`).join("")}</div>
        ${journalError ? renderHint(journalError, "Danger") : ""}
        ${journalLoading ? `<div class="empty-state">正在读取本地期刊库...</div>` : renderJournalTable()}
      </article>
    </section>
  `;
}

function renderJournalTable() {
  if (!journals.length) {
    return `<div class="empty-state">暂无匹配期刊。可调整来源、分区或关键词条件。</div>`;
  }
  return `
    <div class="table-wrap">
      <table>
        <thead><tr><th>期刊</th><th>来源</th><th>等级</th><th>JIF / JCR</th><th>中科院</th><th>OA</th><th>影响因子</th><th>学科</th><th>范围/RAG</th><th>个人经验</th><th>操作</th></tr></thead>
        <tbody>
          ${journals.map((journal) => `
            <tr>
              <td><strong>${escapeHtml(journal.name)}</strong><small>${escapeHtml(journal.issn || journal.eissn)}</small></td>
              <td>${escapeHtml(journal.source_db)}</td>
              <td>${escapeHtml(journal.grade || journal.ami_level || journal.pku_core)}</td>
              <td>${escapeHtml(journal.jif_quartile)}${journal.jcr_rank_detail ? `<small>${escapeHtml(journal.jcr_rank_detail)}</small>` : ""}</td>
              <td>${escapeHtml(journal.cas_quartile || journal.cas_zone)}</td>
              <td>${escapeHtml(truncateText([journal.oa_flag, journal.oa_detail].filter(Boolean).join(" / "), 42))}</td>
              <td>${journal.impact_factor ?? "-"}</td>
              <td>${escapeHtml(journal.subjects)}</td>
              <td>${[journal.scope_text ? "范围" : "", journal.article_topics ? "文章" : "", journal.submission_guidelines ? "指南" : ""].filter(Boolean).join(" / ") || "未补充"}</td>
              <td>${escapeHtml([journal.difficulty, journal.apc, journal.review_cycle, journal.avoid_reason].filter(Boolean).join(" / ") || "未评估")}</td>
              <td class="action-cell"><button class="small" data-edit-journal="${journal.id}">编辑</button><button class="small" data-enrich-journal="${journal.id}">补全</button></td>
            </tr>
          `).join("")}
        </tbody>
      </table>
    </div>
    <div class="pager">
      <button id="journalPrev" ${journalPage === 0 ? "disabled" : ""}>上一页</button>
      <span>第 ${journalPage + 1} / ${Math.max(1, Math.ceil(journalTotal / journalPageSize))} 页，共 ${journalTotal} 本期刊</span>
      <button id="journalNext" ${(journalPage + 1) * journalPageSize >= journalTotal ? "disabled" : ""}>下一页</button>
    </div>
  `;
}

function escapeHtml(value: unknown): string {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

function truncateText(value: string, limit = 48): string {
  const text = String(value ?? "").trim();
  if (text.length <= limit) {
    return text;
  }
  return `${text.slice(0, Math.max(0, limit - 1)).trimEnd()}…`;
}

function renderAttachmentStatus() {
  if (!attachmentStatus) return "";
  const message = "附件状态：已保存 " + escapeHtml(attachmentStatus.storedPath) + "；已提取 " + attachmentStatus.textLength + " 字。" + (attachmentStatus.warning ? "提示：" + escapeHtml(attachmentStatus.warning) + "；" : "") + escapeHtml(attachmentStatus.privacyNote);
  return renderHint(message, attachmentStatus.warning ? "Important" : "Normal");
}

function renderQuickHealthReport(report: any) {
  const items = report.items ?? [];
  return renderHint("快速体检时间：" + escapeHtml(report.checked_at ?? ""), "Normal") + items.map((item: any) => renderHint(escapeHtml(item.name) + "：" + escapeHtml(item.status) + "。" + escapeHtml(item.detail), item.level ?? "Normal")).join("");
}

function applyStrategyPreset(value: string) {
  selectedStrategy = value || "均衡推荐";
  privacyState.localOnly = selectedStrategy === "仅本地";
  privacyState.doNotSendFullText = selectedStrategy === "隐私优先";
  privacyState.aiTitleAbstractOnly = selectedStrategy === "隐私优先";
}

function xfastapiModels() {
  return ["gpt-5.5", "gpt-5.6"];
}

function normalizeModelForService(serviceKey: string, model: string) {
  if (serviceKey === "xfastapi") {
    return xfastapiModels().includes(model) ? model : "gpt-5.6";
  }
  return model || "openrouter/free";
}

function renderAiModelOptions() {
  const models = aiState.serviceKey === "xfastapi"
    ? xfastapiModels()
    : ["openrouter/free", "openai/gpt-4o-mini", "google/gemini-2.0-flash-exp:free"];
  const selectedModel = normalizeModelForService(aiState.serviceKey, aiState.model);
  return models.map((model) => `<option value="${escapeHtml(model)}" ${model === selectedModel ? "selected" : ""}>${escapeHtml(model)}</option>`).join("");
}

function renderAutomaticInputHint() {
  const filled = [state.title, state.abstractText, state.keywords, state.fullText].filter((value) => value.trim()).length;
  if (filled === 0) return renderHint("填写题目、摘要、关键词或正文任意一项即可开始推荐。", "Normal");
  if (filled === 1 && state.title.trim()) return renderHint("当前信息较少，推荐可能不够准确。建议补充摘要、关键词或全文后再推荐。", "Important");
  return renderHint(`已填写 ${filled} 项论文信息，可开始推荐。`, "Normal");
}

function renderAiIndexHint() {
  if (!indexHealth) return "";
  if (indexHealth.status === "正常") {
    return renderHint("RAG/Embedding 索引正常：本地语义相似度、FTS 关键词和规则分会一起参与推荐。", "Normal");
  }
  if (indexHealth.status === "构建中") {
    return renderHint(`RAG/Embedding 索引正在后台构建：${indexHealth.detail} 构建期间可以推荐，但语义相似度可能偏低。`, "Important");
  }
  return renderHint(`RAG/Embedding 索引${indexHealth.status}：${indexHealth.detail}。建议在设置页点击“重建索引”。`, "Important");
}

function renderAiPage() {
  return `
    <section class="panel-grid">
      <article class="panel wide">
        <h3>AI 推荐投稿</h3>
        ${renderHint("题目、摘要、关键词、全文附件任意一项即可推荐；只填题目时必须提示信息不足但不能拦住。", "Important")}
        ${renderHint("AI 推荐可能会把题目、摘要或正文节选发送给所选接口；隐私模式可禁用上传全文。", "Danger")}
        ${renderAiIndexHint()}
        <label>论文题目<input id="titleInput" value="${state.title}" placeholder="只填写题目也允许推荐，但会提示准确度风险" /></label>
        <label>摘要<textarea id="abstractInput" placeholder="建议补充摘要，提高语义匹配质量">${state.abstractText}</textarea></label>
        <label>关键词<input id="keywordsInput" value="${state.keywords}" placeholder="例如：graph neural network, traffic forecasting" /></label>
        <label>正文节选<textarea id="fullTextInput" placeholder="可由 docx/pdf/txt/md 附件自动提取">${state.fullText}</textarea></label>
        <div class="drop-zone" id="attachmentDrop">
          <strong>选择或拖入论文附件</strong>
          <small>支持 DOCX / PDF / TXT / MD。选择或拖入后会自动保存并提取正文；扫描版 PDF 无法提取时可直接粘贴全文。</small>
          <input id="attachmentFile" type="file" accept=".docx,.pdf,.txt,.md" />
        </div>
        <div class="radar-filters compact">
          <select id="aiService">
            <option value="openrouter" ${aiState.serviceKey === "openrouter" ? "selected" : ""}>OpenRouter</option>
            <option value="xfastapi" ${aiState.serviceKey === "xfastapi" ? "selected" : ""}>xFastAPI</option>
          </select>
          <select id="aiBaseUrl" title="接口地址">
            <option value="https://openrouter.ai/api/v1" ${aiState.baseUrl === "https://openrouter.ai/api/v1" ? "selected" : ""}>OpenRouter 地址</option>
            <option value="https://xfastapi.ai" ${aiState.baseUrl === "https://xfastapi.ai" ? "selected" : ""}>xFastAPI 地址</option>
          </select>
          <select id="aiModel" title="模型名称">
            ${renderAiModelOptions()}
          </select>
          <input id="aiApiKey" value="${escapeHtml(aiState.apiKey)}" type="password" placeholder="API Key；页面输入优先，不会保存到项目文件" />
          <button id="loadAiSettings">读取已保存 API</button>
          <button id="saveAiSettings">保存 API</button>
        </div>
         ${renderHint("xFastAPI：模型已固定为 gpt-5.5 / gpt-5.6；接口地址只填 https://xfastapi.ai。若提示模型不支持，请确认密钥分组是否包含对应模型。", "Important")}
        <div class="radar-filters compact">
          <label class="check"><input type="checkbox" id="privacyLocalOnly" ${privacyState.localOnly ? "checked" : ""}/> 仅本地推荐</label>
          <label class="check"><input type="checkbox" id="privacyDisableWeb" ${privacyState.disableWebSearch ? "checked" : ""}/> 禁用联网搜索</label>
          <label class="check"><input type="checkbox" id="privacyNoFullText" ${privacyState.doNotSendFullText ? "checked" : ""}/> 不上传全文</label>
          <label class="check"><input type="checkbox" id="privacyTitleAbstractOnly" ${privacyState.aiTitleAbstractOnly ? "checked" : ""}/> AI 只用题目摘要</label>
        </div>
        ${renderHint("隐私开关会真实影响 AI 和联网行为：仅本地推荐不会调用 AI；禁用联网搜索会阻止联网补充；不上传全文会清空发给 AI 的正文节选。", "Danger")}
        <div class="radar-filters compact">
          <select id="strategyPreset">
            <option value="均衡推荐" ${selectedStrategy === "均衡推荐" ? "selected" : ""}>均衡推荐</option>
            <option value="保守稳妥" ${selectedStrategy === "保守稳妥" ? "selected" : ""}>保守稳妥</option>
            <option value="冲刺高分区" ${selectedStrategy === "冲刺高分区" ? "selected" : ""}>冲刺高分区</option>
            <option value="低版面费优先" ${selectedStrategy === "低版面费优先" ? "selected" : ""}>低版面费优先</option>
            <option value="快审优先" ${selectedStrategy === "快审优先" ? "selected" : ""}>快审优先</option>
            <option value="隐私优先" ${selectedStrategy === "隐私优先" ? "selected" : ""}>隐私优先</option>
            <option value="仅本地" ${selectedStrategy === "仅本地" ? "selected" : ""}>仅本地</option>
          </select>
        </div>
        <div class="filter-group"><span>投稿方向</span><label class="check"><input type="checkbox" data-ai-subject value="自动识别" ${subjectSelection.length === 0 ? "checked" : ""}/> 自动识别</label>${subjectOptions.map((value) => `<label class="check"><input type="checkbox" data-ai-subject value="${value}" ${subjectSelection.includes(value) ? "checked" : ""}/> ${value}</label>`).join("")}</div>
        ${renderHint("投稿方向会作为推荐硬约束：选计算机/智能交通时，历史、文学等明显无关期刊不会进入正常推荐；不选则根据题目自动识别。", "Danger")}
        <div class="filter-group"><span>目标 JIF 分区</span>${["Q1", "Q2", "Q3", "Q4"].map((value) => `<label class="check"><input type="checkbox" data-ai-jif value="${value}" ${targetJifSelection.includes(value) ? "checked" : ""}/> ${value}</label>`).join("")}</div>
        ${renderHint("综合分按 100 分计算；已勾选的 JIF 分区属于硬筛选，不符合条件的期刊不会进入推荐。", "Important")}
        ${renderAttachmentStatus()}
        <div class="toolbar">
          <button class="primary" id="aiRecommend">AI 推荐</button>
          <button id="webSearchMore">联网补充</button>
          <button id="extractAttachment">保存并提取附件</button>
        </div>
        ${renderHint("再次点击 AI 推荐或联网补充，会自动避开当前论文已经展示过的期刊，尽量换一批结果；修改论文信息后会重新从第一批开始。", "Normal")}
        <div id="automaticInputHint">${renderAutomaticInputHint()}</div>
        <div id="recommendationOutput" class="result-list"></div>
      </article>
    </section>
  `;
}

function renderLibraryPage() {
  return `
    <article class="panel wide">
      <h3>我的收藏与个人经验</h3>
      ${renderHint("建议写清难度、审稿周期、版面费、拒稿原因、适合/不适合的论文类型；这些内容会进入 RAG 并影响以后推荐。", "Important")}
      <div class="radar-filters compact">
        <input id="libraryKeyword" value="${escapeHtml(libraryState.keyword)}" placeholder="搜索期刊名、标签、备注或学科" />
        <select id="libraryDifficulty">
          <option value="">全部难度</option>
          ${["未评估", "易", "中", "难", "极难"].map((value) => `<option value="${value}" ${libraryState.difficulty === value ? "selected" : ""}>${value}</option>`).join("")}
        </select>
        <button class="primary" id="searchLibrary">查询</button>
        <button id="exportLibrary">导出 CSV</button>
      </div>
      ${renderLibraryTable()}
    </article>
  `;
}

function renderLibraryTable() {
  const filtered = journals.filter((journal) => {
    const keyword = libraryState.keyword.trim().toLowerCase();
    const text = `${journal.name} ${journal.subjects} ${journal.tags} ${journal.summary} ${journal.experience}`.toLowerCase();
    const keywordOk = !keyword || text.includes(keyword);
    const difficultyOk = !libraryState.difficulty || journal.difficulty === libraryState.difficulty;
    return keywordOk && difficultyOk;
  });
  if (!filtered.length) {
    return `<div class="empty-state">暂无收藏期刊。导入旧版数据或从推荐结果加入收藏后，这里会显示难度、版面费、审稿周期和备注。</div>`;
  }
  return `
    <div class="table-wrap">
      <table>
        <thead><tr><th>期刊</th><th>状态</th><th>难度</th><th>版面费</th><th>审稿周期</th><th>标签</th><th>官网</th><th>备注/经验</th><th>操作</th></tr></thead>
        <tbody>
          ${filtered.map((journal) => `
            <tr>
              <td><strong>${escapeHtml(journal.name)}</strong><small>${escapeHtml(journal.source_db)}</small></td>
              <td>${escapeHtml(journal.recommendation_preference)}</td>
              <td>${escapeHtml(journal.difficulty)}</td>
              <td>${escapeHtml(journal.apc)}</td>
              <td>${escapeHtml(journal.review_cycle)}</td>
              <td>${escapeHtml(journal.tags)}</td>
              <td>${journal.website ? `<span>${escapeHtml(journal.website)}</span>` : "-"}</td>
              <td>${escapeHtml([journal.summary, journal.experience, journal.avoid_reason ? `不推荐：${journal.avoid_reason}` : ""].filter(Boolean).join("；"))}</td>
              <td class="action-cell"><button class="small" data-edit-journal="${journal.id}">编辑</button><button class="small danger-button" data-delete-journal="${journal.id}">删除</button></td>
            </tr>
          `).join("")}
        </tbody>
      </table>
    </div>
  `;
}

function renderSubmissionsPage() {
  return `
    <article class="panel wide">
      <h3>投稿记录</h3>
      ${renderHint("投稿结果、拒稿原因、审稿周期会反哺期刊个人经验库。", "Important")}
      <div class="radar-filters compact">
        <input id="submissionKeyword" value="${escapeHtml(submissionState.keyword)}" placeholder="搜索题目、期刊、作者或备注" />
        <select id="submissionStatus">
          <option value="">全部状态</option>
          ${["准备中", "已投", "修回", "接收", "拒稿"].map((value) => `<option value="${value}" ${submissionState.status === value ? "selected" : ""}>${value}</option>`).join("")}
        </select>
        <button class="primary" id="searchSubmissions">查询</button>
        <button id="addSubmission">新增记录</button>
        <button id="exportSubmissions">导出 CSV</button>
      </div>
      ${submissionError ? renderHint(submissionError, "Danger") : ""}
      ${renderSubmissionTable()}
    </article>
  `;
}

function renderSubmissionTable() {
  if (!submissions.length) {
    return `<div class="empty-state">暂无投稿记录。导入旧版数据后，这里会显示论文投稿状态、版面费和经验备注。</div>`;
  }
  return `
    <div class="table-wrap">
      <table>
        <thead><tr><th>论文题目</th><th>期刊</th><th>状态</th><th>投稿日期</th><th>结果日期</th><th>版面费</th><th>作者</th><th>备注/经验</th><th>操作</th></tr></thead>
        <tbody>
          ${submissions.map((item) => `
            <tr>
              <td><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.created_at)}</small></td>
              <td>${escapeHtml(item.journal_name)}</td>
              <td>${escapeHtml(item.status)}</td>
              <td>${escapeHtml(item.submit_date)}</td>
              <td>${escapeHtml(item.result_date)}</td>
              <td>${escapeHtml(item.apc_paid)}</td>
              <td>${escapeHtml(item.authors || item.corresponding)}</td>
              <td>${escapeHtml(item.notes)}</td>
              <td class="action-cell"><button class="small" data-edit-submission="${item.id}">编辑</button><button class="small danger-button" data-delete-submission="${item.id}">删除</button></td>
            </tr>
          `).join("")}
        </tbody>
      </table>
    </div>
  `;
}

function renderTemplatesPage() {
  return `
    <article class="panel wide">
      <h3>投稿模板</h3>
      ${renderHint("已有模板可按期刊搜索和分页查看；新增同名模板会提示并跳过，不覆盖你的原文件。", "Important")}
      <div class="radar-filters compact">
        <input id="templateSearch" placeholder="搜索期刊名或模板文件名" />
        <button class="primary" id="searchTemplates">搜索模板</button>
      </div>
      <div class="template-list">${templateItems.length ? templateItems.map((item) => `<div class="result-card"><h4>${escapeHtml(item.journal_name)}</h4><p>${escapeHtml(item.file_name)}</p></div>`).join("") : `<div class="empty-state">暂无已关联的期刊模板。可在下方添加，或先复制通用模板。</div>`}</div>
      <div class="pager"><button id="templatePrev" ${templatePage === 0 ? "disabled" : ""}>上一页</button><span>第 ${templatePage + 1} 页，共 ${templateTotal} 个模板</span><button id="templateNext" ${(templatePage + 1) * templatePageSize >= templateTotal ? "disabled" : ""}>下一页</button></div>
      <div class="section-rule"></div>
      <div class="radar-filters compact">
        <input id="templateJournalName" placeholder="输入期刊名称，例如 IEEE ACCESS" />
        <button class="primary" id="copyTemplates">复制通用模板</button>
      </div>
      <div class="drop-zone" id="templateDrop"><strong>选择或拖入期刊模板文件</strong><small>先填写关联期刊名称，再上传 Word、PDF、TXT 或其他模板文件。</small><input id="templateFile" type="file" /></div>
      <div class="toolbar dense"><button class="primary" id="saveJournalTemplate">添加期刊模板</button></div>
    </article>
  `;
}

function renderImportPage() {
  return `
    <article class="panel wide">
      <h3>资源导入</h3>
      ${renderHint("导入前会自动备份数据库；原始文件保存到 data\raw_resources，并可关联到你填写的期刊。", "Important")}
      ${renderHint("填写期刊名称后上传该期刊的资料，资料会被保存并关联进期刊档案；批量目录可留空期刊名称。", "Important")}
      <label>关联期刊名称<input id="resourceJournalName" placeholder="例如：IEEE ACCESS；批量目录可留空" /></label>
      <div class="drop-zone" id="resourceDrop"><strong>选择或拖入资源文件</strong><small>支持 Excel / CSV / PDF。无需填写本地路径。</small><input id="resourceFile" type="file" accept=".xlsx,.xls,.csv,.pdf" /></div>
      <label>资源类型<input id="resourceTypeInput" value="期刊目录" placeholder="例如：JCR / CSSCI / CCF / 自定义资源" /></label>
      <label>列名列表（可选）<input id="resourceColumnsInput" placeholder="可填写 Journal Name, ISSN, Impact Factor；也可留空后再识别" /></label>
      <label>备注<textarea id="resourceNotesInput" placeholder="导入说明、数据来源、版本日期等"></textarea></label>
      <div class="toolbar dense"><button class="primary" id="saveResourceFile">导入资源文件</button><button id="detectColumns">识别列名</button></div>
      <div id="importOutput" class="result-list"></div>
    </article>
  `;
}

function renderSettingsPage() {
  return `
    <section class="panel-grid">
      <article class="panel">
        <h3>诊断</h3>
        <dl class="facts">
          <dt>数据目录</dt><dd>${diagnostics?.root ?? "未初始化"}</dd>
          <dt>数据库</dt><dd>${diagnostics?.database ?? "未初始化"}</dd>
          <dt>Schema</dt><dd>${diagnostics?.schema_version ?? "-"}</dd>
          <dt>索引</dt><dd>${indexHealth?.status ?? diagnostics?.index_status ?? "-"}</dd>
        </dl>
        ${(diagnostics?.notes ?? []).map((note) => renderHint(note, "Normal")).join("")}
      </article>
      <article class="panel">
        <h3>索引健康检查</h3>
        ${renderHint(indexHealth?.detail ?? "索引状态等待初始化。", "Important")}
        <button id="checkIndex">刷新索引状态</button>
        <button id="rebuildIndex">重建索引</button>
        <button id="clearIndexCache">清理索引缓存</button>
        ${renderHint("修改备注、版面费、审稿周期或导入新数据后，软件会自动标记索引过期；下次推荐前会增量刷新，也可以手动重建或清理缓存。", "Normal")}
      </article>
      <article class="panel">
        <h3>安全与隐私</h3>
        ${hints.map((hint) => renderHint(hint.text, hint.level)).join("")}
        <button id="backupNow">立即备份数据库</button>
        <button id="exportAllData">导出全部数据</button>
        <button id="listBackups">查看备份列表</button>
        <button class="primary" id="quickHealthCheck">快速体检</button><div id="quickHealthOutput">${quickHealthHtml}</div><button id="openDataDirectory">打开数据文件夹</button>
        ${renderHint("从备份恢复会覆盖同名数据文件。软件会先自动创建 pre_restore 备份，但仍建议只恢复你确认过的备份目录。", "Danger")}
        <label>备份目录路径<input id="restoreBackupPath" placeholder="例如 D:\\投刊机器人\\data\\backups\\backup_20260730_manual" /></label>
        <button class="danger-button" id="restoreBackup">从备份恢复</button>
        ${renderHint("迁移旧版前请填写旧版项目目录，例如 C:\\Users\\Administrator\\Desktop\\投刊助手源码。新版会先备份 data，再跳过重复期刊和投稿记录。", "Important")}
        <label>旧版项目目录<input id="legacyRootInput" value="C:\\Users\\Administrator\\Desktop\\投刊助手源码" /></label>
        <button id="importLegacy">导入旧版数据</button>
      </article>
    </section>
  `;
}

function bindPageEvents() {
  document.querySelector<HTMLButtonElement>("#searchJournals")?.addEventListener("click", async () => {
    radarState.keyword = document.querySelector<HTMLInputElement>("#radarKeyword")?.value ?? "";
    radarState.source = document.querySelector<HTMLSelectElement>("#radarSource")?.value ?? "";
    radarState.jif = checkedValues("[data-radar-jif]");
    radarState.cas = checkedValues("[data-radar-cas]");
    journalPage = 0;
    radarState.ccf = document.querySelector<HTMLInputElement>("#radarCcf")?.checked ?? false;
    radarState.ei = document.querySelector<HTMLInputElement>("#radarEi")?.checked ?? false;
    await loadJournals();
    renderShell();
  });
  document.querySelector<HTMLButtonElement>("#journalPrev")?.addEventListener("click", async () => { journalPage = Math.max(0, journalPage - 1); await loadJournals(); renderShell(); });
  document.querySelector<HTMLButtonElement>("#journalNext")?.addEventListener("click", async () => { journalPage += 1; await loadJournals(); renderShell(); });
  document.querySelector<HTMLButtonElement>("#searchSubmissions")?.addEventListener("click", async () => {
    submissionState.keyword = document.querySelector<HTMLInputElement>("#submissionKeyword")?.value ?? "";
    submissionState.status = document.querySelector<HTMLSelectElement>("#submissionStatus")?.value ?? "";
    await loadSubmissions();
    renderShell();
  });
  document.querySelector<HTMLButtonElement>("#searchLibrary")?.addEventListener("click", () => {
    libraryState.keyword = document.querySelector<HTMLInputElement>("#libraryKeyword")?.value ?? "";
    libraryState.difficulty = document.querySelector<HTMLSelectElement>("#libraryDifficulty")?.value ?? "";
    renderShell();
  });
  document.querySelector<HTMLButtonElement>("#exportLibrary")?.addEventListener("click", async () => {
    libraryState.keyword = document.querySelector<HTMLInputElement>("#libraryKeyword")?.value ?? "";
    libraryState.difficulty = document.querySelector<HTMLSelectElement>("#libraryDifficulty")?.value ?? "";
    const report = await invoke<any>("export_journals_csv", {
      filter: {
        keyword: libraryState.keyword,
        source_db: [],
        jif_quartile: [],
        cas_zone: [],
        ccf: [],
        ei_only: false,
        limit: 1000,
      },
    });
    alert(`${report.message}\n共 ${report.rows_exported} 条\n${report.file_path}`);
  });
  document.querySelectorAll<HTMLButtonElement>("[data-edit-journal]").forEach((button) => {
    button.addEventListener("click", async () => {
      const journal = journals.find((item) => item.id === Number(button.dataset.editJournal));
      if (!journal) return;
      await editJournalProfile(journal);
    });
  });
  document.querySelectorAll<HTMLButtonElement>("[data-enrich-journal]").forEach((button) => {
    button.addEventListener("click", async () => {
      const journal = journals.find((item) => item.id === Number(button.dataset.enrichJournal));
      if (!journal) return;
      await enrichJournal(journal.id, journal.name);
    });
  });
  document.querySelectorAll<HTMLButtonElement>("[data-delete-journal]").forEach((button) => {
    button.addEventListener("click", async () => {
      const journal = journals.find((item) => item.id === Number(button.dataset.deleteJournal));
      if (!journal) return;
      if (!confirm(`确定删除收藏期刊「${journal.name}」吗？删除前建议先备份。`)) return;
      await invoke("delete_journal", { journalId: journal.id });
      await loadJournals();
      indexHealth = await invoke<IndexHealth>("get_index_health");
      renderShell();
    });
  });
  document.querySelector<HTMLButtonElement>("#addSubmission")?.addEventListener("click", async () => {
    await editSubmission();
  });
  document.querySelector<HTMLButtonElement>("#exportSubmissions")?.addEventListener("click", async () => {
    submissionState.keyword = document.querySelector<HTMLInputElement>("#submissionKeyword")?.value ?? "";
    submissionState.status = document.querySelector<HTMLSelectElement>("#submissionStatus")?.value ?? "";
    const report = await invoke<any>("export_submissions_csv", {
      filter: {
        keyword: submissionState.keyword,
        status: submissionState.status ? [submissionState.status] : [],
        limit: 1000,
      },
    });
    alert(`${report.message}\n共 ${report.rows_exported} 条\n${report.file_path}`);
  });
  document.querySelectorAll<HTMLButtonElement>("[data-edit-submission]").forEach((button) => {
    button.addEventListener("click", async () => {
      const submission = submissions.find((item) => item.id === Number(button.dataset.editSubmission));
      if (!submission) return;
      await editSubmission(submission);
    });
  });
  document.querySelectorAll<HTMLButtonElement>("[data-delete-submission]").forEach((button) => {
    button.addEventListener("click", async () => {
      const submission = submissions.find((item) => item.id === Number(button.dataset.deleteSubmission));
      if (!submission) return;
      if (!confirm(`确定删除投稿记录「${submission.title}」吗？删除前建议先备份。`)) return;
      await invoke("delete_submission", { submissionId: submission.id });
      await loadSubmissions();
      renderShell();
    });
  });
  document.querySelector<HTMLInputElement>("#attachmentFile")?.addEventListener("change", async (event) => {
    attachmentFile = ((event.currentTarget as HTMLInputElement).files ?? [])[0] ?? null;
    if (attachmentFile) await uploadAttachmentFile(attachmentFile);
  });
  bindDropZone("#attachmentDrop", async (file) => { attachmentFile = file; await uploadAttachmentFile(file); });
  document.querySelector<HTMLInputElement>("#resourceFile")?.addEventListener("change", (event) => { resourceFile = ((event.currentTarget as HTMLInputElement).files ?? [])[0] ?? null; });
  bindDropZone("#resourceDrop", (file) => { resourceFile = file; return Promise.resolve(); });
  document.querySelector<HTMLInputElement>("#templateFile")?.addEventListener("change", (event) => { templateFile = ((event.currentTarget as HTMLInputElement).files ?? [])[0] ?? null; });
  bindDropZone("#templateDrop", (file) => { templateFile = file; return Promise.resolve(); });
  document.querySelector<HTMLButtonElement>("#searchTemplates")?.addEventListener("click", async () => { templatePage = 0; await loadTemplates(document.querySelector<HTMLInputElement>("#templateSearch")?.value ?? ""); renderShell(); });
  document.querySelector<HTMLButtonElement>("#templatePrev")?.addEventListener("click", async () => { templatePage = Math.max(0, templatePage - 1); await loadTemplates(document.querySelector<HTMLInputElement>("#templateSearch")?.value ?? ""); renderShell(); });
  document.querySelector<HTMLButtonElement>("#templateNext")?.addEventListener("click", async () => { templatePage += 1; await loadTemplates(document.querySelector<HTMLInputElement>("#templateSearch")?.value ?? ""); renderShell(); });
  document.querySelector<HTMLButtonElement>("#saveJournalTemplate")?.addEventListener("click", async () => {
    const journalName = document.querySelector<HTMLInputElement>("#templateJournalName")?.value.trim() ?? "";
    if (!journalName || !templateFile) { alert("请填写关联期刊名称并选择模板文件。"); return; }
    const report = await invoke<any>("upload_journal_template", { journalName, fileName: templateFile.name, bytes: await fileBytes(templateFile) });
    alert(report.message); await loadTemplates(""); renderShell();
  });
  document.querySelector<HTMLButtonElement>("#copyTemplates")?.addEventListener("click", async () => {
    const journalName = document.querySelector<HTMLInputElement>("#templateJournalName")?.value.trim() ?? "";
    if (!journalName) {
      alert("请先输入期刊名称。");
      return;
    }
    const report = await invoke<any>("copy_generic_templates", { journalName });
    alert(`${report.message}\n复制 ${report.copied} 个，跳过 ${report.skipped} 个\n${report.target_dir}`);
  });

  document.querySelector<HTMLSelectElement>("#strategyPreset")?.addEventListener("change", (event) => {
    collectAiInputs();
    applyStrategyPreset((event.currentTarget as HTMLSelectElement).value);
    renderShell();
  });
  document.querySelector<HTMLSelectElement>("#aiService")?.addEventListener("change", (event) => {
    const serviceKey = (event.currentTarget as HTMLSelectElement).value;
    aiState.serviceKey = serviceKey;
    aiState.baseUrl = serviceKey === "xfastapi" ? "https://xfastapi.ai" : "https://openrouter.ai/api/v1";
    aiState.model = serviceKey === "xfastapi" ? "gpt-5.6" : "openrouter/free";
    renderShell();
  });
  document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("#titleInput, #abstractInput, #keywordsInput, #fullTextInput").forEach((input) => {
    input.addEventListener("input", () => {
      collectAiInputs();
      const target = document.querySelector<HTMLDivElement>("#automaticInputHint");
      if (target) target.innerHTML = renderAutomaticInputHint();
    });
  });
  document.querySelectorAll<HTMLInputElement>("[data-ai-subject]").forEach((input) => {
    input.addEventListener("change", () => {
      if (input.value === "自动识别" && input.checked) {
        document.querySelectorAll<HTMLInputElement>("[data-ai-subject]").forEach((item) => {
          if (item.value !== "自动识别") item.checked = false;
        });
      } else if (input.checked) {
        const auto = Array.from(document.querySelectorAll<HTMLInputElement>("[data-ai-subject]")).find((item) => item.value === "自动识别");
        if (auto) auto.checked = false;
      }
      collectSubjectSelection();
    });
  });
  document.querySelector<HTMLButtonElement>("#aiRecommend")?.addEventListener("click", async () => {
    collectAiInputs();
    collectAiServiceInputs();
    collectPrivacyInputs();
    targetJifSelection = checkedValues("[data-ai-jif]");
    collectSubjectSelection();
    resetRecommendationRefreshIfInputChanged();
    const excludedIds = [...seenRecommendationIds];
    const excludedNames = [...seenRecommendationNames];
    const currentRound = recommendationRefreshRound;
    if (!privacyState.localOnly && !aiState.apiKey.trim()) {
      const saved = await invoke<any>("load_ai_settings", { serviceKey: aiState.serviceKey });
      aiState.serviceKey = saved.service_key || aiState.serviceKey;
      aiState.baseUrl = saved.base_url || aiState.baseUrl;
      aiState.model = normalizeModelForService(aiState.serviceKey, saved.model || aiState.model);
      aiState.apiKey = saved.api_key || aiState.apiKey;
    }
    try {
    const response = await invoke<any>("ai_recommend", {
      request: {
        ai: {
          service_key: aiState.serviceKey,
          base_url: aiState.baseUrl,
          model: aiState.model,
          api_key: aiState.apiKey,
          messages: [],
          temperature: 0.15,
          max_tokens: 1800,
        },
        recommendation: {
          title: state.title,
          abstract_text: state.abstractText,
          keywords: state.keywords,
          full_text: state.fullText,
          preferences: aiRecommendationPreferences(),
          journal_type: "系统推荐",
          target_zone: "系统推荐",
          target_jif_quartiles: targetJifSelection,
          strategy: selectedStrategy,
          privacy_mode: privacyState.localOnly,
          disable_web_search: privacyState.disableWebSearch,
          do_not_send_full_text: privacyState.doNotSendFullText,
          ai_title_abstract_only: privacyState.aiTitleAbstractOnly,
          exclude_journal_ids: excludedIds,
          exclude_journal_names: excludedNames,
          refresh_round: currentRound,
        },
      },
    });
    const target = document.querySelector<HTMLDivElement>("#recommendationOutput");
    if (target) {
      lastRecommendationItems = response.items ?? [];
      rememberRecommendationItems(lastRecommendationItems);
      recommendationRefreshRound += 1;
      target.innerHTML = `${response.warning ? renderHint(response.warning, "Important") : ""}${renderRecommendationItems(lastRecommendationItems)}${response.raw_ai_text ? `<div class="result-card"><h4>AI 解释</h4><p>${escapeHtml(response.raw_ai_text)}</p></div>` : ""}`;
      bindRecommendationActionEvents();
    }
    } catch (error) {
      const items = await invoke<any[]>("local_fallback_recommend", {
        request: {
          title: state.title,
          abstract_text: state.abstractText,
          keywords: state.keywords,
          full_text: state.fullText,
          preferences: aiRecommendationPreferences(),
          journal_type: "系统推荐",
          target_zone: "系统推荐",
          target_jif_quartiles: targetJifSelection,
          strategy: selectedStrategy,
          privacy_mode: privacyState.localOnly,
          disable_web_search: privacyState.disableWebSearch,
          do_not_send_full_text: privacyState.doNotSendFullText,
          ai_title_abstract_only: privacyState.aiTitleAbstractOnly,
          exclude_journal_ids: excludedIds,
          exclude_journal_names: excludedNames,
          refresh_round: currentRound,
        },
      });
      lastRecommendationItems = items ?? [];
      rememberRecommendationItems(lastRecommendationItems);
      recommendationRefreshRound += 1;
      const target = document.querySelector<HTMLDivElement>("#recommendationOutput");
      if (target) {
        target.innerHTML = `${renderHint(`AI 接口不可用，已改用本地推荐：${String(error)}`, "Important")}${renderRecommendationItems(lastRecommendationItems)}`;
        bindRecommendationActionEvents();
      }
    }
  });
  document.querySelector<HTMLButtonElement>("#loadAiSettings")?.addEventListener("click", async () => {
    collectAiServiceInputs();
    const typedKey = aiState.apiKey.trim();
    const record = await invoke<any>("load_ai_settings", { serviceKey: aiState.serviceKey });
    aiState.serviceKey = record.service_key || aiState.serviceKey;
    aiState.baseUrl = record.base_url || aiState.baseUrl;
    aiState.model = normalizeModelForService(aiState.serviceKey, record.model || aiState.model);
    aiState.apiKey = record.api_key || typedKey;
    renderShell();
    if (record.api_key) {
      alert(`已读取已保存 API 设置：${record.api_key_hint}`);
    } else if (typedKey) {
      alert("没有读取到已保存的 API Key；已保留当前输入框里的 Key。需要长期使用请点“加密保存当前 API”。");
    } else {
      alert("没有读取到已保存的 API Key。请先输入 Key，然后点“加密保存当前 API”，也可以直接点 AI 推荐临时使用当前输入。");
    }
  });
  document.querySelector<HTMLButtonElement>("#saveAiSettings")?.addEventListener("click", async () => {
    collectAiServiceInputs();
    const record = await invoke<any>("save_ai_settings", {
      record: {
        service_key: aiState.serviceKey,
        base_url: aiState.baseUrl,
        model: aiState.model,
        api_key: aiState.apiKey,
        api_key_hint: "",
      },
    });
    aiState.apiKey = record.api_key || aiState.apiKey;
    renderShell();
    alert(`API 设置已加密保存：${record.api_key_hint}`);
  });
  document.querySelector<HTMLButtonElement>("#webSearchMore")?.addEventListener("click", async () => {
    collectAiInputs();
    collectPrivacyInputs();
    targetJifSelection = checkedValues("[data-ai-jif]");
    collectSubjectSelection();
    resetRecommendationRefreshIfInputChanged();
    const excludedTitles = [...seenWebTitles];
    const currentRound = webRefreshRound;
    if (privacyState.disableWebSearch || privacyState.localOnly) {
      alert("隐私模式已阻止联网补充。本地推荐仍可正常使用。");
      return;
    }
    const report = await invoke<any>("web_search_sources", {
      request: {
        title: state.title,
        abstract_text: state.abstractText,
        keywords: state.keywords,
        prefer_chinese: false,
        exclude_titles: excludedTitles,
        refresh_round: currentRound,
        target_jif_quartiles: targetJifSelection,
      },
    });
    const target = document.querySelector<HTMLDivElement>("#recommendationOutput");
    if (target) {
      rememberWebSources(report.sources ?? []);
      webRefreshRound += 1;
      target.innerHTML = `${report.warning ? renderHint(report.warning, "Important") : ""}${renderWebSources(report.sources ?? [])}`;
      bindExternalUrlEvents();
    }
  });
  document.querySelector<HTMLButtonElement>("#extractAttachment")?.addEventListener("click", async () => {
    if (!attachmentFile) { alert("请选择或拖入论文附件；也可以直接粘贴全文。"); return; }
    const report = await invoke<any>("upload_attachment", { fileName: attachmentFile.name, bytes: await fileBytes(attachmentFile) });
    const extractedLength = String(report.extracted_text ?? "").length;
    if (report.extracted_text) {
      state.fullText = report.extracted_text;
    }
    attachmentStatus = {
      storedPath: String(report.stored_path ?? ""),
      textLength: extractedLength,
      warning: String(report.warning ?? ""),
      privacyNote: privacyState.doNotSendFullText ? "当前设置为不上传全文，AI 不会使用附件正文。" : "当前 AI 推荐会使用正文节选，隐私模式可关闭上传。",
    };
    alert(`附件已保存：${report.stored_path}${report.warning ? `\n提示：${report.warning}` : ""}`);
    renderShell();
  });
  document.querySelector<HTMLButtonElement>("#detectColumns")?.addEventListener("click", async () => {
    const columns = (document.querySelector<HTMLInputElement>("#resourceColumnsInput")?.value ?? "")
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
    const detected = await invoke<Record<string, string>>("detect_resource_columns", { columns });
    const target = document.querySelector<HTMLDivElement>("#importOutput");
    if (target) {
      target.innerHTML = `<div class="result-card"><h4>列名识别结果</h4><pre>${escapeHtml(JSON.stringify(detected, null, 2))}</pre></div>`;
    }
  });
  document.querySelector<HTMLButtonElement>("#saveResourceFile")?.addEventListener("click", async () => {
    if (!resourceFile) { alert("请选择或拖入资源文件。"); return; }
    const columns = (document.querySelector<HTMLInputElement>("#resourceColumnsInput")?.value ?? "")
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
    const columnMap = await invoke<Record<string, string>>("detect_resource_columns", { columns });
    const report = await invoke<any>("upload_resource", {
      fileName: resourceFile.name,
      bytes: await fileBytes(resourceFile),
      resourceType: document.querySelector<HTMLInputElement>("#resourceTypeInput")?.value ?? "",
      journalName: document.querySelector<HTMLInputElement>("#resourceJournalName")?.value ?? "",
      notes: document.querySelector<HTMLTextAreaElement>("#resourceNotesInput")?.value ?? "",
    });
    /* legacy request retained for source compatibility
      request: {
        source_path: "",
        resource_type: document.querySelector<HTMLInputElement>("#resourceTypeInput")?.value ?? "",
        column_map: columnMap,
        notes: document.querySelector<HTMLTextAreaElement>("#resourceNotesInput")?.value ?? "",
      },
    }); */
    alert(`${report.message}\n保存位置：${report.stored_path}\n备份位置：${report.backup_dir ?? "-"}`);
  });
  document.querySelector<HTMLButtonElement>("#checkIndex")?.addEventListener("click", async () => {
    indexHealth = await invoke<IndexHealth>("get_index_health");
    renderShell();
  });
  document.querySelector<HTMLButtonElement>("#rebuildIndex")?.addEventListener("click", async () => {
    const rebuilt = await invoke<IndexHealth>("rebuild_rag_index");
    indexHealth = rebuilt;
    renderShell();
    alert(`索引已重建：${rebuilt.status}\n${rebuilt.detail}\n待更新 chunk：${rebuilt.dirty_chunks}\n索引字节：${rebuilt.index_bytes}`);
  });
  document.querySelector<HTMLButtonElement>("#clearIndexCache")?.addEventListener("click", async () => {
    if (!confirm("确定清理索引缓存吗？不会删除期刊、附件、模板或投稿记录，但下次推荐前需要重新生成索引。")) return;
    const cleared = await invoke<IndexHealth>("clear_index_cache");
    indexHealth = cleared;
    renderShell();
    alert(`索引缓存已清理：${cleared.status}\n${cleared.detail}\n索引字节：${cleared.index_bytes}`);
  });
  document.querySelector<HTMLButtonElement>("#backupNow")?.addEventListener("click", async () => {
    const report = await invoke<any>("create_backup", { reason: "manual" });
    alert(report.message + "\n" + report.backup_dir);
  });
  document.querySelector<HTMLButtonElement>("#exportAllData")?.addEventListener("click", async () => {
    const report = await invoke<any>("export_all_data");
    alert(`${report.message}\n共 ${report.files_copied} 个文件\n${report.backup_dir}`);
  });
  document.querySelector<HTMLButtonElement>("#listBackups")?.addEventListener("click", async () => {
    const backups = await invoke<any[]>("list_backups");
    if (!backups.length) {
      alert("还没有备份。");
      return;
    }
    alert(backups.map((item) => `${item.name}\n${item.path}`).join("\n\n"));
  });
  document.querySelector<HTMLButtonElement>("#quickHealthCheck")?.addEventListener("click", async () => {
    const report = await invoke<any>("quick_health_check");
    quickHealthHtml = renderQuickHealthReport(report);
    renderShell();
  });
  document.querySelector<HTMLButtonElement>("#openDataDirectory")?.addEventListener("click", async () => {
    const path = await invoke<string>("open_data_directory");
    alert(`已打开数据文件夹：\n${path}`);
  });
  document.querySelector<HTMLButtonElement>("#restoreBackup")?.addEventListener("click", async () => {
    const backupPath = document.querySelector<HTMLInputElement>("#restoreBackupPath")?.value.trim() ?? "";
    if (!backupPath) {
      alert("请填写备份目录路径。");
      return;
    }
    const confirmation = prompt("此操作会覆盖同名数据文件。请输入“恢复”继续。");
    if (confirmation !== "恢复") {
      alert("已取消恢复。");
      return;
    }
    const report = await invoke<any>("restore_from_backup", { backupPath });
    alert(`${report.message}\n恢复 ${report.files_restored} 个文件\n恢复前备份：${report.pre_restore_backup_dir}`);
    await initialize();
  });
  document.querySelector<HTMLButtonElement>("#importLegacy")?.addEventListener("click", async () => {
    const sourceRoot = document.querySelector<HTMLInputElement>("#legacyRootInput")?.value.trim() ?? "";
    if (!sourceRoot) {
      alert("请填写旧版项目目录。");
      return;
    }
    const report = await invoke<any>("import_legacy_data", { sourceRoot });
    alert(
      `迁移完成：新增期刊 ${report.journals_imported} 条，更新 ${report.journals_updated} 条，投稿记录 ${report.submissions_imported} 条。` +
      `\n报告：${report.report_file ?? "-"}`
    );
    await loadJournals();
    renderShell();
  });
}

function renderRecommendationItems(items: any[]) {
  if (!items.length) {
    return `${renderHint("本地库暂未找到匹配期刊，不是软件故障。可调整分区条件、补充关键词，或使用“联网补充”继续查找。", "Important")}<div class="empty-state">当前筛选条件下没有可推荐的本地期刊。系统不会用无关期刊凑结果。</div>`;
  }
  const referenceHint = items.some((item) => item.reference_only)
    ? renderHint("本地库没有高匹配期刊，以下为低匹配参考期刊，仅供参考；请重点核验学科范围与投稿要求。", "Important")
    : "";
  const qualityHint = items.some((item) => Number(item.keyword_score ?? 0) === 0 || Number(item.semantic_score ?? 0) < 0.3)
    ? renderHint("准确度提示：部分结果关键词分或语义分偏低。建议补充论文英文关键词，并在期刊编辑页补充官网 Aims & Scope / 近期文章主题，可明显提高向量匹配质量。", "Important")
    : "";
  return referenceHint + qualityHint + items.map((item, index) => `
    <div class="result-card">
      <strong>${escapeHtml(item.fit_level)}</strong>
      <h4>${escapeHtml(item.journal_name)}</h4><strong class="score">综合分 ${Number(item.total_score ?? 0).toFixed(1)} / 100</strong>
      ${item.reference_only ? renderHint("低匹配参考：本地库暂未找到高匹配期刊，这条只用于拓展备选范围，请人工核验学科范围。", "Important") : ""}
      <p>${escapeHtml(item.reason)}</p>
      <small>
        来源：${escapeHtml(item.source)}
        · 语义相似度：${Number(item.semantic_score ?? 0).toFixed(3)}
        · 关键词分：${Number(item.keyword_score ?? 0).toFixed(2)}
        · 规则分：${Number(item.rule_score ?? 0).toFixed(2)}
      </small>
      <p><small>${escapeHtml(item.personal_experience_effect)}</small></p>
      <p><small>${escapeHtml(item.evidence)}</small></p>
      <small>${escapeHtml(item.risk)}</small>
      <div class="toolbar dense result-actions">
        <button data-rec-action="favorite" data-rec-index="${index}">加入收藏</button>
        <button data-rec-action="less" data-rec-index="${index}">少推荐</button>
        <button class="danger-button" data-rec-action="never" data-rec-index="${index}">不推荐</button>
        <button data-rec-action="submission" data-rec-index="${index}">记录投稿经历</button>
        ${item.website ? `<button data-rec-action="website" data-rec-index="${index}">打开官网</button>` : ""}
      </div>
    </div>
  `).join("");
}

function recommendationJournalFilter(keyword: string) {
  return { keyword, source_db: [], jif_quartile: [], cas_zone: [], ccf: [], ei_only: false, limit: 10 };
}

function bindRecommendationActionEvents() {
  document.querySelectorAll<HTMLButtonElement>("[data-rec-action]").forEach((button) => {
    button.addEventListener("click", async () => {
      const index = Number(button.dataset.recIndex ?? -1);
      const action = button.dataset.recAction ?? "";
      const item = lastRecommendationItems[index];
      if (!item) return;
      if (action === "website") {
        if (item.website) await invoke("open_external_url", { url: item.website });
        return;
      }
      if (action === "submission") {
        await editSubmission({
          id: 0,
          title: state.title,
          authors: "",
          corresponding: "",
          journal_name: item.journal_name,
          submit_date: new Date().toISOString().slice(0, 10),
          status: "准备中",
          result_date: "",
          apc_paid: "",
          website: item.website ?? "",
          notes: "来自推荐结果：" + (item.fit_level ?? "") + "；" + (item.reason ?? ""),
          created_at: "",
        });
        return;
      }
      const journalId = Number(item.journal_id ?? 0);
      if (!journalId) {
        alert("这个结果不是本地期刊库中的正式候选，不能直接写入个人经验。");
        return;
      }
      if (action === "favorite") {
        const records = await invoke<JournalRecord[]>("list_journals", { filter: recommendationJournalFilter(item.journal_name) });
        const journal = records.find((entry) => entry.id === journalId) ?? records[0];
        if (journal) await editJournalProfile(journal);
        else alert("没有找到本地期刊记录，请先刷新期刊库后再试。");
        return;
      }
      if (action === "less" || action === "never") {
        const preference = action === "less" ? "少推荐" : "不推荐";
        await invoke("set_recommendation_preference", { journalId, preference });
        await loadJournals();
        indexHealth = await invoke<IndexHealth>("get_index_health");
        alert("已标记为" + preference + "：" + item.journal_name + "。以后推荐会考虑这个个人经验。");
      }
    });
  });
}
function renderWebSources(items: any[]) {
  if (!items.length) {
    return `<div class="empty-state">联网补充没有找到可用结果。本地推荐不受影响。</div>`;
  }
  return items.map((item) => `<div class="result-card"><strong>${escapeHtml(item.provider)}</strong><h4>${escapeHtml(item.title)}</h4>${item.jif_quartile ? `<strong class="score">本地库核验 JIF：${escapeHtml(item.jif_quartile)}</strong>` : ""}<p>${escapeHtml(item.snippet)}</p><small>以上期刊名称与 ISSN 来自 Crossref 记录；分区、影响因子和收录情况仍需人工核验。</small></div>`).join("");
}

function bindExternalUrlEvents() {
  document.querySelectorAll<HTMLButtonElement>("[data-web-url]").forEach((button) => {
    button.addEventListener("click", async () => {
      const url = button.dataset.webUrl ?? "";
      if (!url) return;
      try {
        await invoke("open_external_url", { url });
      } catch (error) {
        alert(`无法打开链接：${String(error)}`);
      }
    });
  });
}

function renderActiveModal() {
  if (editingJournal) return renderJournalProfileModal(editingJournal);
  if (editingSubmission !== undefined) return renderSubmissionModal(editingSubmission ?? null);
  return "";
}

function renderOptions(values: string[], selected: string) {
  return values
    .map((value) => `<option value="${escapeHtml(value)}" ${value === selected ? "selected" : ""}>${escapeHtml(value)}</option>`)
    .join("");
}

function renderJournalProfileModal(journal: JournalRecord) {
  return `
    <div class="modal-backdrop">
      <form class="modal-card" id="journalProfileForm">
        <header class="modal-head">
          <div>
            <h3>编辑期刊经验</h3>
            <p>${escapeHtml(journal.name)}</p>
          </div>
          <button type="button" class="small" id="closeModal">取消</button>
        </header>
        ${renderHint("难度、推荐偏好、版面费、审稿周期和投稿经验会进入 RAG 知识库，并影响以后推荐排序。", "Important")}
        ${renderHint("期刊研究范围 / Aims & Scope 是提高向量推荐准确度的关键字段；建议粘贴官网的收稿范围摘要，不要自己编。", "Important")}
        ${(journal.oa_flag || journal.oa_detail || journal.jcr_rank_detail)
          ? renderHint(`自动采集：${[journal.oa_flag, journal.jcr_rank_detail].filter(Boolean).join(" · ") || "已读取"}${journal.oa_detail ? `；${journal.oa_detail}` : ""}`, "Normal")
          : ""}
        <div class="form-grid">
          <label>难度
            <select name="difficulty">${renderOptions(["未评估", "易", "中", "难", "极难"], journal.difficulty || "未评估")}</select>
          </label>
          <label>推荐偏好
            <select name="recommendation_preference">${renderOptions(["正常推荐", "少推荐", "不推荐", "适合保底", "适合冲刺"], journal.recommendation_preference || "正常推荐")}</select>
          </label>
          <label>版面费
            <input name="apc" value="${escapeHtml(journal.apc)}" placeholder="例如：2400 USD / 约 8000 元 / 无版面费" />
          </label>
          <label>审稿周期
            <input name="review_cycle" value="${escapeHtml(journal.review_cycle)}" placeholder="例如：2-3 个月；一审 45 天" />
          </label>
          <label>标签
            <input name="tags" value="${escapeHtml(journal.tags)}" placeholder="例如：快审、版面费高、适合综述、避坑" />
          </label>
          <label>官网/投稿网址
            <input name="website" value="${escapeHtml(journal.website)}" placeholder="请填完整网址，后续核验和模板会用到" />
          </label>
        </div>
        <label>期刊研究范围 / Aims & Scope
          <textarea name="scope_text" placeholder="建议粘贴官网 aims & scope 或收稿范围摘要，例如：publishes research on traffic forecasting, intelligent transportation systems, graph learning...">${escapeHtml(journal.scope_text)}</textarea>
        </label>
        <label>近期文章主题 / Article Topics
          <textarea name="article_topics" placeholder="建议记录近期论文标题关键词，例如：traffic flow prediction; graph neural networks; spatio-temporal forecasting">${escapeHtml(journal.article_topics)}</textarea>
        </label>
        <label>投稿指南摘要 / Guidelines
          <textarea name="submission_guidelines" placeholder="例如：要求 IEEE 模板；正文长度限制；图表格式；数据/代码可用性声明；是否接受综述">${escapeHtml(journal.submission_guidelines)}</textarea>
        </label>
        <label>推荐原因 / 总结备注
          <textarea name="summary" placeholder="例如：与研究主题匹配、收录稳定、适合作为保底或冲刺">${escapeHtml(journal.summary)}</textarea>
        </label>
        <div class="form-grid">
          <label>适合论文主题
            <input name="suitable_topics" value="${escapeHtml(journal.suitable_topics)}" placeholder="例如：图神经网络、数字治理、马克思主义理论" />
          </label>
          <label>拒稿原因 / 风险记录
            <input name="rejection_reason" value="${escapeHtml(journal.rejection_reason)}" placeholder="例如：超出范围、创新性不足、格式不符" />
          </label>
        </div>
        <label>不推荐原因
          <textarea name="avoid_reason" placeholder="例如：版面费过高、审稿太慢、实际难度高、不适合纯理论文章">${escapeHtml(journal.avoid_reason)}</textarea>
        </label>
        <label>投稿经验 / 避坑提醒
          <textarea name="experience" placeholder="例如：官网说快但实际一审很慢；版面费上涨；不适合纯理论论文">${escapeHtml(journal.experience)}</textarea>
        </label>
        ${renderHint("保存后会自动标记索引过期；下次推荐前会增量刷新，不需要你改代码。", "Normal")}
        <footer class="modal-actions">
          <button type="button" id="closeModalSecondary">取消</button>
          <button class="primary" type="submit">保存并更新记忆</button>
        </footer>
      </form>
    </div>
  `;
}

function renderSubmissionModal(existing: SubmissionRecord | null) {
  const today = new Date().toISOString().slice(0, 10);
  return `
    <div class="modal-backdrop">
      <form class="modal-card" id="submissionForm">
        <header class="modal-head">
          <div>
            <h3>${existing ? "编辑投稿记录" : "新增投稿记录"}</h3>
            <p>投稿状态、拒稿原因、版面费和周期会帮助你以后判断期刊真实难度。</p>
          </div>
          <button type="button" class="small" id="closeModal">取消</button>
        </header>
        ${renderHint("题目是必填项；备注里建议写清拒稿原因、审稿体验、版面费变化和是否还值得推荐。", "Important")}
        <label>论文题目 *
          <input name="title" required value="${escapeHtml(existing?.title ?? "")}" placeholder="请输入论文题目" />
        </label>
        <div class="form-grid">
          <label>期刊名称
            <input name="journal_name" value="${escapeHtml(existing?.journal_name ?? "")}" placeholder="投稿或计划投稿的期刊" />
          </label>
          <label>状态
            <select name="status">${renderOptions(["准备中", "已投", "修回", "接收", "拒稿"], existing?.status ?? "准备中")}</select>
          </label>
          <label>投稿日期
            <input name="submit_date" value="${escapeHtml(existing?.submit_date ?? today)}" placeholder="YYYY-MM-DD" />
          </label>
          <label>结果日期
            <input name="result_date" value="${escapeHtml(existing?.result_date ?? "")}" placeholder="YYYY-MM-DD，可为空" />
          </label>
          <label>作者
            <input name="authors" value="${escapeHtml(existing?.authors ?? "")}" />
          </label>
          <label>通信作者
            <input name="corresponding" value="${escapeHtml(existing?.corresponding ?? "")}" />
          </label>
          <label>版面费
            <input name="apc_paid" value="${escapeHtml(existing?.apc_paid ?? "")}" placeholder="实际支付或询价结果" />
          </label>
          <label>投稿系统/期刊网址
            <input name="website" value="${escapeHtml(existing?.website ?? "")}" />
          </label>
        </div>
        <label>备注 / 经验
          <textarea name="notes" placeholder="例如：拒稿原因、审稿周期、编辑态度、是否建议下次再投">${escapeHtml(existing?.notes ?? "")}</textarea>
        </label>
        <footer class="modal-actions">
          <button type="button" id="closeModalSecondary">取消</button>
          <button class="primary" type="submit">保存记录</button>
        </footer>
      </form>
    </div>
  `;
}

function formText(form: FormData, name: string) {
  return String(form.get(name) ?? "").trim();
}

function closeModal() {
  editingJournal = null;
  editingSubmission = undefined;
  renderShell();
}

function bindModalEvents() {
  document.querySelector<HTMLButtonElement>("#closeModal")?.addEventListener("click", closeModal);
  document.querySelector<HTMLButtonElement>("#closeModalSecondary")?.addEventListener("click", closeModal);

  document.querySelector<HTMLFormElement>("#journalProfileForm")?.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!editingJournal) return;
    const data = new FormData(event.currentTarget as HTMLFormElement);
    const journal = editingJournal;
    await invoke("update_journal_profile", {
      update: {
        journal_id: journal.id,
        difficulty: formText(data, "difficulty"),
        recommendation_preference: formText(data, "recommendation_preference"),
        apc: formText(data, "apc"),
        review_cycle: formText(data, "review_cycle"),
        tags: formText(data, "tags"),
        website: formText(data, "website"),
        scope_text: formText(data, "scope_text"),
        summary: formText(data, "summary"),
        experience: formText(data, "experience"),
        rejection_reason: formText(data, "rejection_reason"),
        suitable_topics: formText(data, "suitable_topics"),
        avoid_reason: formText(data, "avoid_reason"),
        article_topics: formText(data, "article_topics"),
        submission_guidelines: formText(data, "submission_guidelines"),
      },
    });
    editingJournal = null;
    await loadJournals();
    indexHealth = await invoke<IndexHealth>("get_index_health");
    renderShell();
  });

  document.querySelector<HTMLFormElement>("#submissionForm")?.addEventListener("submit", async (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget as HTMLFormElement);
    const title = formText(data, "title");
    if (!title) return;
    await invoke("save_submission", {
      submission: {
        id: editingSubmission?.id && editingSubmission.id > 0 ? editingSubmission.id : null,
        title,
        authors: formText(data, "authors"),
        corresponding: formText(data, "corresponding"),
        journal_name: formText(data, "journal_name"),
        submit_date: formText(data, "submit_date"),
        status: formText(data, "status"),
        result_date: formText(data, "result_date"),
        apc_paid: formText(data, "apc_paid"),
        website: formText(data, "website"),
        notes: formText(data, "notes"),
      },
    });
    editingSubmission = undefined;
    await loadSubmissions();
    renderShell();
  });
}

async function editJournalProfile(journal: JournalRecord) {
  editingJournal = journal;
  editingSubmission = undefined;
  renderShell();
}

async function editSubmission(existing?: SubmissionRecord) {
  editingSubmission = existing ?? null;
  editingJournal = null;
  renderShell();
}

async function enrichJournal(journalId: number, journalName: string) {
  try {
    const report = await invoke<any>("enrich_journals", {
      request: {
        journal_ids: [journalId],
        dry_run: false,
      },
    });
    await loadJournals();
    indexHealth = await invoke<IndexHealth>("get_index_health");
    renderShell();
    const lineItems = (report.journals ?? [])
      .flatMap((item: any) => (item.fields ?? []).map((field: any) => `${field.field_name}：${field.status}（${field.message}）`));
    alert(
      [`已补全：${journalName}`, `处理期刊：${report.processed ?? 0}`, `更新字段：${report.updated_fields ?? 0}`, `跳过字段：${report.skipped_fields ?? 0}`, ...lineItems.slice(0, 8)].join("\n")
    );
  } catch (error) {
    alert(`补全失败：${journalName}\n${String(error)}`);
  }
}

async function loadSubmissions() {
  submissionError = "";
  try {
    submissions = await invoke<SubmissionRecord[]>("list_submissions", {
      filter: {
        keyword: submissionState.keyword,
        status: submissionState.status ? [submissionState.status] : [],
        limit: 100,
      },
    });
  } catch (error) {
    submissionError = `投稿记录读取失败：${String(error)}`;
  }
}

async function loadJournals() {
  journalLoading = true;
  journalError = "";
  try {
    journals = await invoke<JournalRecord[]>("list_journals", {
      filter: {
        keyword: radarState.keyword,
        source_db: radarState.source ? [radarState.source] : [],
        jif_quartile: radarState.jif,
        cas_zone: radarState.cas,
        ccf: radarState.ccf ? ["CCF"] : [],
        ei_only: radarState.ei,
        limit: journalPageSize,
        offset: journalPage * journalPageSize,
      },
    });
    journalTotal = await invoke<number>("count_journals", {
      filter: {
        keyword: radarState.keyword,
        source_db: radarState.source ? [radarState.source] : [],
        jif_quartile: radarState.jif,
        cas_zone: radarState.cas,
        ccf: radarState.ccf ? ["CCF"] : [],
        ei_only: radarState.ei,
        limit: 1,
      },
    });
    if (journalPage > 0 && journals.length === 0 && journalTotal > 0) {
      journalPage = Math.max(0, Math.ceil(journalTotal / journalPageSize) - 1);
      await loadJournals();
    }
  } catch (error) {
    journalError = `期刊库读取失败：${String(error)}`;
  } finally {
    journalLoading = false;
  }
}

function checkedValues(selector: string): string[] {
  return Array.from(document.querySelectorAll<HTMLInputElement>(selector)).filter((item) => item.checked).map((item) => item.value);
}

function collectSubjectSelection() {
  const values = checkedValues("[data-ai-subject]");
  const concrete = values.filter((value) => value !== "自动识别");
  subjectSelection = concrete;
}

function aiRecommendationPreferences() {
  const preferences = ["稳妥"];
  if (subjectSelection.length > 0) {
    preferences.push(`投稿方向:${subjectSelection.join("|")}`);
  }
  return preferences;
}

function collectAiInputs() {
  state.title = document.querySelector<HTMLInputElement>("#titleInput")?.value ?? "";
  state.abstractText = document.querySelector<HTMLTextAreaElement>("#abstractInput")?.value ?? "";
  state.keywords = document.querySelector<HTMLInputElement>("#keywordsInput")?.value ?? "";
  state.fullText = document.querySelector<HTMLTextAreaElement>("#fullTextInput")?.value ?? "";
}

function currentRecommendationInputKey() {
  return [state.title, state.abstractText, state.keywords, state.fullText, targetJifSelection.join("|"), subjectSelection.join("|"), selectedStrategy]
    .map((value) => value.trim())
    .join("\n---\n");
}

function resetRecommendationRefreshIfInputChanged() {
  const nextKey = currentRecommendationInputKey();
  if (nextKey !== recommendationInputKey) {
    recommendationInputKey = nextKey;
    recommendationRefreshRound = 0;
    webRefreshRound = 0;
    seenRecommendationIds = [];
    seenRecommendationNames = [];
    seenWebTitles = [];
  }
}

function rememberRecommendationItems(items: any[]) {
  const ids = new Set(seenRecommendationIds);
  const names = new Set(seenRecommendationNames);
  for (const item of items) {
    const id = Number(item.journal_id ?? 0);
    if (id > 0) ids.add(id);
    const name = String(item.journal_name ?? "").trim();
    if (name) names.add(name);
  }
  seenRecommendationIds = Array.from(ids);
  seenRecommendationNames = Array.from(names);
}

function rememberWebSources(items: any[]) {
  const titles = new Set(seenWebTitles);
  for (const item of items) {
    const title = String(item.title ?? "").trim();
    if (title) titles.add(title);
  }
  seenWebTitles = Array.from(titles);
}

function collectAiServiceInputs() {
  aiState.serviceKey = document.querySelector<HTMLSelectElement>("#aiService")?.value ?? "openrouter";
  aiState.baseUrl = document.querySelector<HTMLSelectElement>("#aiBaseUrl")?.value ?? "";
  aiState.model = normalizeModelForService(aiState.serviceKey, document.querySelector<HTMLSelectElement>("#aiModel")?.value ?? "");
  aiState.apiKey = document.querySelector<HTMLInputElement>("#aiApiKey")?.value ?? "";
  if (aiState.serviceKey === "xfastapi") {
    const suppliedUrl = aiState.baseUrl.trim();
    aiState.baseUrl = suppliedUrl
      .replace(/\/(v1\/responses|responses|v1\/chat\/completions|chat\/completions|v1)\/?$/i, "")
      .replace(/\/$/, "") || "https://xfastapi.ai";
  }
}

function collectPrivacyInputs() {
  privacyState.localOnly = document.querySelector<HTMLInputElement>("#privacyLocalOnly")?.checked ?? false;
  privacyState.disableWebSearch = document.querySelector<HTMLInputElement>("#privacyDisableWeb")?.checked ?? false;
  privacyState.doNotSendFullText = document.querySelector<HTMLInputElement>("#privacyNoFullText")?.checked ?? false;
  privacyState.aiTitleAbstractOnly = document.querySelector<HTMLInputElement>("#privacyTitleAbstractOnly")?.checked ?? false;
}

async function fileBytes(file: File): Promise<number[]> { return Array.from(new Uint8Array(await file.arrayBuffer())); }
function bindDropZone(selector: string, onFile: (file: File) => Promise<void>) {
  const zone = document.querySelector<HTMLElement>(selector); if (!zone) return;
  zone.addEventListener("dragover", (event) => { event.preventDefault(); zone.classList.add("dragging"); });
  zone.addEventListener("dragleave", () => zone.classList.remove("dragging"));
  zone.addEventListener("drop", async (event) => { event.preventDefault(); zone.classList.remove("dragging"); const file = event.dataTransfer?.files?.[0]; if (file) await onFile(file); });
}
async function uploadAttachmentFile(file: File) {
  const report = await invoke<any>("upload_attachment", { fileName: file.name, bytes: await fileBytes(file) });
  if (report.extracted_text) state.fullText = report.extracted_text;
  attachmentStatus = { storedPath: String(report.stored_path ?? ""), textLength: String(report.extracted_text ?? "").length, warning: String(report.warning ?? ""), privacyNote: "附件已保存；也可直接粘贴全文。" };
  renderShell();
}
async function loadTemplates(query = "") {
  const page = await invoke<any>("list_journal_templates", { query, offset: templatePage * templatePageSize, limit: templatePageSize });
  templateItems = page.items ?? []; templateTotal = Number(page.total ?? 0);
}

async function initialize() {
  diagnostics = await invoke<DiagnosticSummary>("initialize_app");
  indexHealth = await invoke<IndexHealth>("get_index_health");
  hints = await invoke<Hint[]>("get_default_hints");
  await loadJournals();
  await loadSubmissions();
  await loadTemplates();
  renderShell();
}

renderShell();
initialize().catch((error) => {
  console.error(error);
  document.querySelector<HTMLDivElement>("#app")!.innerHTML = `<div class="fatal"><h1>投刊机器人初始化失败</h1><pre>${String(error)}</pre></div>`;
});
