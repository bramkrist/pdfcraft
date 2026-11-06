// INDIE Impose — impose.pixelindie.com. Everything runs in this browser: the PDF never leaves
// the computer. The engine is pdfcraft-impose compiled to WebAssembly (bramkrist/pdfcraft,
// apps/impose-web); pages are drawn with pdf.js for the picture preview only.
//
// Work happens in tabs: the file you open is the first tab, and every layout you apply becomes a
// new tab you can lay out again (n-up on A4, then step & repeat of that onto A3+), view, and save.

const root = document.getElementById('app');
const user = root.dataset.user || '';
const A = (f) => '/a.php?f=' + encodeURIComponent(f);

const SHEETS = {
  'A3+ 32×48': [320, 480],
  '33×48': [330, 480],
  'SRA3': [320, 450],
  'A3': [297, 420],
  'A4': [210, 297],
};
const MODES = [
  ['repeat', 'Step & Repeat'],
  ['sequential', 'N-up'],
  ['cut-stack', 'Unique Collate'],
  ['booklet', 'Booklet'],
  ['bleed', 'Bleed'],
];
const MODE_TAG = { repeat: 'STEPREPEAT', sequential: 'NUP', 'cut-stack': 'UNIQUECOLLATE', booklet: 'BOOKLET', bleed: 'BLEED' };

const state = {
  engine: null,
  docs: [], // { id, name, bytes, sizes, tags: [] }
  active: null,
  mode: 'repeat',
  view: 'layout', // 'doc' | 'layout' | 'picture'
  layout: null,
  sheet: 0,
  busy: false,
  seq: 0,
};

root.innerHTML = `
<header class="top">
  <div class="brand"><div class="disc" aria-hidden="true">P</div><div class="word">IMPOSE</div></div>
  <div class="tag">PIXELINDIE · Pre-press</div>
  <div class="spacer"></div>
  <div class="who"><span id="ver"></span> · ${esc(user)}<a href="/logout.php">Keluar</a></div>
</header>
<nav class="tabs" id="tabs" aria-label="Dokumen"></nav>
<div class="layout">
  <aside>
    <div id="nobanner" class="banner" hidden></div>
    <section class="panel">
      <h2>File</h2>
      <label class="drop" id="drop"><strong>Buka PDF</strong><span>Pilih atau tarik ke sini. Diproses di komputer ini, tidak di-upload.</span><input id="file" type="file" accept="application/pdf,.pdf"></label>
      <div class="fileinfo" id="fileinfo"></div>
    </section>
    <section class="panel" id="pfpanel" hidden>
      <h2>Cek file (preflight)</h2>
      <div id="pf"></div>
    </section>
    <section class="panel">
      <h2>Layout berikutnya</h2>
      <div class="seg" id="modes">${MODES.map(([k, t]) => `<button type="button" data-mode="${k}" aria-pressed="${k === state.mode}">${t}</button>`).join('')}</div>
    </section>
    <section class="panel" id="opts"></section>
  </aside>
  <section class="stage">
    <div class="msg" id="msg" role="status"></div>
    <div class="stagehead">
      <div class="summary" id="summary"></div>
      <div class="viewseg" id="views">
        <button type="button" data-view="doc">Isi tab</button>
        <button type="button" data-view="layout">Layout garis</button>
        <button type="button" data-view="picture">Layout gambar</button>
      </div>
    </div>
    <div class="canvaswrap"><canvas id="cv" width="900" height="600"></canvas></div>
    <div class="nav"><button type="button" id="prev" aria-label="Sebelumnya">‹</button><span id="pos"></span><button type="button" id="next" aria-label="Berikutnya">›</button></div>
    <div class="actions">
      <label class="check ack" id="ackwrap" hidden><input type="checkbox" id="ack"> File ada masalah merah. Saya sudah cek dan tetap lanjut.</label>
      <button type="button" class="btn" id="apply" disabled>Terapkan ke tab baru</button>
      <button type="button" class="btn ghost" id="download" disabled>Download tab ini</button>
      <button type="button" class="btn ghost" id="saveas" disabled hidden>Simpan ke folder…</button>
    </div>
  </section>
</div>`;

const $ = (id) => document.getElementById(id);

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}
function fmt(n, d = 1) {
  return Number(n).toLocaleString('id-ID', { maximumFractionDigits: d });
}
const activeDoc = () => state.docs.find((d) => d.id === state.active) || null;

// ---- options ------------------------------------------------------------------------------
const num = (k, label, value, attrs = '') => `<div><label class="l" for="o-${k}">${label}</label><input type="number" id="o-${k}" data-k="${k}" value="${value}" step="any" ${attrs}></div>`;
const sel = (k, label, opts, value, cls = '') => `<div class="${cls}"><label class="l" for="o-${k}">${label}</label><select id="o-${k}" data-k="${k}">${opts.map(([v, t]) => `<option value="${v}" ${v === value ? 'selected' : ''}>${t}</option>`).join('')}</select></div>`;
const chk = (k, label, on) => `<label class="check full"><input type="checkbox" id="o-${k}" data-k="${k}" ${on ? 'checked' : ''}>${label}</label>`;

function sheetFields(defaultSheet = 'A3+ 32×48', withAuto = false) {
  return `
    ${sel('sheet', 'Kertas', [...(withAuto ? [['auto', 'Otomatis (2× halaman)']] : []), ...Object.keys(SHEETS).map((k) => [k, k]), ['custom', 'Custom']], defaultSheet, 'full')}
    <div class="full row2" id="customsheet" hidden>${num('sheet_w', 'Lebar (mm)', 320)}${num('sheet_h', 'Tinggi (mm)', 480)}</div>
    ${withAuto ? '' : sel('orient', 'Arah', [['best', 'Terbaik (paling banyak up)'], ['landscape', 'Landscape'], ['portrait', 'Portrait']], 'best')}
    ${withAuto ? '' : num('margin', 'Margin (mm)', 5, 'min="0"')}`;
}

function markFields() {
  return `
    <h2 class="sub2">Crop mark</h2>
    <div class="field">
      ${chk('marks', 'Pakai crop mark', true)}
      ${sel('mstyle', 'Jenis', [['corner', 'Sudut'], ['japanese', 'Jepang (trim + bleed)']], 'corner')}
      ${sel('mcolour', 'Warna', [['registration', 'Registration (semua plat)'], ['black', 'Hitam (plat K)']], 'registration')}
      ${num('mlen', 'Panjang (mm)', 5, 'min="0.1"')}
      ${num('moff', 'Jarak dari potong (mm)', 3)}
      ${num('mwidth', 'Tebal (pt)', 0.5, 'min="0.05"')}
      <p class="hint full">Jarak minus = masuk ke gambar. Contoh −3 dengan panjang 5: 3 mm di dalam, 2 mm di luar.</p>
    </div>`;
}

function gridFields(withCopies) {
  return `
    ${num('cols', 'Kolom', '', 'min="1" placeholder="auto"')}
    ${num('rows', 'Baris', '', 'min="1" placeholder="auto"')}
    ${num('gx', 'Jarak kolom (mm)', 0, 'min="0"')}
    ${num('gy', 'Jarak baris (mm)', 0, 'min="0"')}
    ${num('bleed', 'Bleed di file (mm)', 0, 'min="0"')}
    ${sel('duplex', 'Sisi cetak', [['', '1 sisi'], ['long-edge', '2 sisi · balik sisi panjang'], ['short-edge', '2 sisi · balik sisi pendek']], '')}
    ${withCopies ? num('copies', 'Ulang blok', 2, 'min="1"') : ''}
    ${state.mode === 'repeat' ? chk('rotate', 'Putar sebagian baris kalau muat lebih banyak', true) : ''}`;
}

function renderOptions() {
  let body = '';
  if (state.mode === 'repeat' || state.mode === 'sequential' || state.mode === 'cut-stack') {
    body = `<h2>Susunan</h2><div class="field">${sheetFields()}${gridFields(state.mode === 'cut-stack')}</div>
      ${state.mode === 'cut-stack' ? '<p class="hint">Tiap posisi jadi satu tumpukan yang urut dari lembar ke lembar. Kolom otomatis dibulatkan ke kelipatan "Ulang blok".</p>' : ''}
      ${markFields()}`;
  } else if (state.mode === 'booklet') {
    body = `<h2>Booklet</h2><div class="field">
      ${sel('binding', 'Jilid', [['saddle', 'Steples tengah'], ['perfect', 'Lem panas (signature)'], ['continuous', 'Berurutan']], 'saddle', 'full')}
      ${num('signature', 'Halaman / signature', 16, 'min="4" step="4"')}
      ${sel('last', 'Signature terakhir', [['multiple-of-four', 'Kelipatan 4'], ['full', 'Penuh']], 'multiple-of-four')}
      ${sheetFields('auto', true)}
      ${sel('align', 'Posisi', [['centre', 'Tengah'], ['spine', 'Rapat ke punggung']], 'centre')}
      ${num('edge', 'Tepi kosong (mm)', 0, 'min="0"')}
      ${chk('scale', 'Skala menyesuaikan kertas', true)}
      ${chk('rtl', 'Jilid di kanan', false)}
    </div>`;
  } else if (state.mode === 'bleed') {
    body = `<h2>Bleed</h2><div class="field">
      ${num('add', 'Bleed (mm)', 3, 'min="0"')}
      ${sel('method', 'Cara', [['mirror', 'Cermin tepi'], ['scale', 'Perbesar halaman']], 'mirror')}
    </div><p class="hint">Tepi halaman sekarang dianggap garis potong. Hasilnya punya TrimBox dan BleedBox.</p>`;
  }
  $('opts').innerHTML = body;
  const fb = fileBleed(activeDoc());
  const bleedInput = document.querySelector('[data-k="bleed"]');
  if (fb !== null && bleedInput) bleedInput.value = fb;
  $('opts').querySelectorAll('[data-k]').forEach((el) => el.addEventListener('input', recompute));
  recompute();
}

function val(k) {
  const el = document.querySelector(`[data-k="${k}"]`);
  if (!el) return undefined;
  if (el.type === 'checkbox') return el.checked;
  return el.value;
}
const numOr = (k, d) => {
  const v = val(k);
  if (v === undefined || v === '') return d;
  const n = Number(v);
  return Number.isFinite(n) ? n : d;
};

function sheetMm() {
  const s = val('sheet');
  const custom = s === 'custom';
  const cs = document.getElementById('customsheet');
  if (cs) cs.hidden = !custom;
  if (s === 'auto' || s === undefined) return null;
  let [w, h] = custom ? [numOr('sheet_w', 320), numOr('sheet_h', 480)] : SHEETS[s];
  const wantLandscape = (val('orient') ?? 'landscape') !== 'portrait';
  if (wantLandscape !== w > h) [w, h] = [h, w];
  return [w, h];
}

function job() {
  const j = { mode: state.mode };
  if (state.mode === 'bleed') {
    j.add_bleed_mm = numOr('add', 3);
    j.method = val('method');
    return j;
  }
  const sheet = sheetMm();
  if (state.mode === 'booklet') {
    if (sheet) j.sheet_mm = sheet[0] >= sheet[1] ? sheet : [sheet[1], sheet[0]];
    j.binding = val('binding');
    if (j.binding === 'perfect') {
      j.signature = numOr('signature', 16);
      j.last_signature = val('last');
    }
    j.align = val('align');
    j.edge_mm = numOr('edge', 0);
    j.scale = val('scale');
    j.right_to_left = val('rtl');
    return j;
  }
  j.sheet_mm = sheet;
  if (val('orient') === 'best') j.sheet_orientation = 'best';
  if (state.mode === 'repeat' && val('rotate')) j.rotate = true;
  j.margin_mm = numOr('margin', 5);
  const cols = numOr('cols', null), rows = numOr('rows', null);
  if (cols) j.cols = cols;
  if (rows) j.rows = rows;
  j.gutter_mm = [numOr('gx', 0), numOr('gy', 0)];
  j.bleed_mm = numOr('bleed', 0);
  if (val('duplex')) j.duplex = val('duplex');
  if (val('marks')) {
    j.marks = {
      length_mm: numOr('mlen', 5),
      offset_mm: numOr('moff', 3),
      style: val('mstyle'),
      width_pt: numOr('mwidth', 0.5),
      registration: val('mcolour') === 'registration',
    };
  }
  if (state.mode === 'cut-stack') j.copies = numOr('copies', 1);
  return j;
}

function jobLabel(j) {
  const m = MODES.find(([k]) => k === j.mode)[1];
  const sheetName = j.sheet_mm ? (Object.entries(SHEETS).find(([, v]) => (v[0] === j.sheet_mm[0] && v[1] === j.sheet_mm[1]) || (v[0] === j.sheet_mm[1] && v[1] === j.sheet_mm[0])) || [`${j.sheet_mm[0]}×${j.sheet_mm[1]}`])[0] : '';
  return `${m}${sheetName ? ' ' + sheetName : ''}`;
}

// ---- engine and pdf.js --------------------------------------------------------------------
async function loadEngine() {
  try {
    const mod = await import(A('pdfcraft_impose_web.js'));
    await mod.default({ module_or_path: A('pdfcraft_impose_web_bg.wasm') });
    state.engine = mod;
    $('ver').textContent = 'engine ' + mod.version();
  } catch (e) {
    const b = $('nobanner');
    b.hidden = false;
    b.textContent = 'Mesin impose belum terpasang di server ini.';
    console.error(e);
  }
}

let pdfjs = null;
async function loadPdfjs() {
  if (!pdfjs) {
    pdfjs = await import(A('pdf.min.mjs'));
    pdfjs.GlobalWorkerOptions.workerSrc = A('pdf.worker.min.mjs');
  }
  return pdfjs;
}
const pdfCache = new WeakMap();
async function pdfDoc(bytes) {
  if (!pdfCache.has(bytes)) {
    const lib = await loadPdfjs();
    pdfCache.set(bytes, lib.getDocument({ data: bytes.slice(), isEvalSupported: false }).promise);
  }
  return pdfCache.get(bytes);
}

// ---- documents (tabs) ---------------------------------------------------------------------
function addDoc(name, bytes, tags = []) {
  let sizes = null;
  try {
    sizes = JSON.parse(state.engine.pageSizes(bytes.slice()));
  } catch (e) {
    say(String(e.message || e), true);
    return null;
  }
  let pf = null;
  try {
    pf = JSON.parse(state.engine.preflight(bytes.slice()));
  } catch (e) {
    console.warn('preflight', e);
  }
  const d = { id: ++state.seq, name, bytes, sizes, tags, pf };
  state.docs.push(d);
  state.active = d.id;
  renderTabs();
  showDocInfo();
  return d;
}

function renderTabs() {
  $('tabs').innerHTML = state.docs.map((d) => `
    <div class="tab ${d.id === state.active ? 'on' : ''}" data-id="${d.id}">
      <button type="button" class="tabname" data-id="${d.id}" title="${esc(d.name)}">${esc(d.name)}</button>
      <button type="button" class="tabx" data-close="${d.id}" aria-label="Tutup tab">×</button>
    </div>`).join('');
}

const STATUS_TEXT = { pass: 'OK', info: 'Info', na: '–', warn: 'Perhatian', fail: 'Masalah' };

function renderPreflight(d) {
  const panel = $('pfpanel');
  if (!d || !d.pf) {
    panel.hidden = true;
    return;
  }
  panel.hidden = false;
  const pf = d.pf;
  const r = pf.report;
  const ink = (on, name, cls) => `<span class="ink ${cls} ${on ? 'on' : 'off'}" title="${name}${on ? ' dipakai' : ' tidak dipakai'}">${name[0]}</span>`;
  const spots = r.inks.spots.map((s) => `<li><span class="ink spot on"></span>${esc(s.name)} <span class="hint">hal. ${s.pages.join(', ')}</span></li>`).join('');
  const rows = pf.checks.map((c) => `<li class="pf-${c.status}"><span class="dot"></span><div><b>${esc(c.title)}</b> <span class="st">${STATUS_TEXT[c.status] || c.status}</span><br><span class="pfmsg">${esc(c.message)}</span></div></li>`).join('');
  const fonts = r.fonts.map((f) => `<li>${f.embedded ? '' : '<b class="bad">TIDAK TERTANAM</b> '}${esc(f.name)} <span class="hint">${esc(f.subtype)}${f.subset ? ' · subset' : ''}</span></li>`).join('');
  const imgs = r.images.slice(0, 50).map((i) => `<li class="${i.missing || i.ppi < 200 ? 'bad' : i.ppi < 300 ? 'meh' : ''}">hal. ${i.page} · ${i.missing ? esc(i.missing) : `${Math.round(i.ppi)} DPI · ${i.width_px}×${i.height_px} px · ${fmt(i.size_mm[0], 0)}×${fmt(i.size_mm[1], 0)} mm · ${esc(i.colour)}`}</li>`).join('');
  $('pf').innerHTML = `
    <div class="pfhead pf-${pf.overall}"><span class="dot"></span>${pf.overall === 'pass' ? 'Siap cetak' : pf.overall === 'warn' ? 'Ada yang perlu dicek' : 'Ada masalah'}</div>
    <div class="inks">${ink(r.inks.cyan, 'Cyan', 'c')}${ink(r.inks.magenta, 'Magenta', 'm')}${ink(r.inks.yellow, 'Yellow', 'y')}${ink(r.inks.black, 'Black', 'k')}
      ${r.inks.rgb ? '<span class="tagchip warn">RGB</span>' : ''}${r.inks.registration ? '<span class="tagchip warn">Registration</span>' : ''}${r.inks.lab ? '<span class="tagchip warn">Lab</span>' : ''}</div>
    ${spots ? `<ul class="plain spots">${spots}</ul>` : ''}
    <ul class="pflist">${rows}</ul>
    ${r.fonts.length ? `<div class="fix"><button type="button" class="btn ghost small" id="do-outline">Outline semua teks</button><span class="hint">Jadi tab baru. Font tertanam diubah jadi kurva, tampilan sama. Font yang tidak tertanam tidak bisa di-outline.</span></div>` : ''}
    ${fonts ? `<details><summary>Font (${r.fonts.length})</summary><ul class="plain">${fonts}</ul></details>` : ''}
    ${imgs ? `<details><summary>Gambar (${r.images.length})</summary><ul class="plain">${imgs}</ul></details>` : ''}`;
}

/** The bleed every page of the file already has, when the file says so on every side. */
function fileBleed(d) {
  const b = d && d.pf ? d.pf.report.pages.map((p) => p.bleed_mm) : [];
  if (!b.length || b.some((x) => !x)) return null;
  const min = Math.min(...b.flat());
  return min > 0.05 ? Math.round(min * 10) / 10 : null;
}

async function outlineActive() {
  const d = activeDoc();
  if (!d || !state.engine) return;
  state.busy = true;
  draw();
  say('Mengubah teks jadi outline…');
  await new Promise((r) => setTimeout(r, 20));
  try {
    const t0 = performance.now();
    const res = state.engine.outlineText(d.bytes.slice());
    const rep = JSON.parse(res.report);
    const bytes = res.bytes;
    if (!rep.outlined) {
      say(rep.kept.length ? `Tidak ada teks yang bisa di-outline: ${rep.kept.map((k) => `hal. ${k.page} ${k.reason}`).slice(0, 4).join('; ')}` : 'Tidak ada teks untuk di-outline.', !!rep.kept.length);
      return;
    }
    const nd = addDoc(`${d.name} › Outline`, bytes, [...d.tags, 'OUTLINE']);
    if (nd) {
      state.view = 'doc';
      state.sheet = 0;
      const kept = rep.kept.length ? ` ${rep.kept.length} bagian dibiarkan sebagai teks: ${[...new Set(rep.kept.map((k) => `hal. ${k.page} (${k.reason})`))].slice(0, 4).join(', ')}.` : '';
      say(`Outline selesai: ${rep.outlined} blok teks, ${rep.glyphs} huruf, ${fmt((performance.now() - t0) / 1000, 1)} detik.${kept}`, !!rep.kept.length, !rep.kept.length);
    }
  } catch (e) {
    say('Outline gagal: ' + String(e.message || e), true);
  } finally {
    state.busy = false;
    recompute();
  }
}

function showDocInfo() {
  const d = activeDoc();
  renderPreflight(d);
  $('ack').checked = false;
  const fb = fileBleed(d);
  const bleedInput = document.querySelector('[data-k="bleed"]');
  if (fb !== null && bleedInput) bleedInput.value = fb;
  if (!d) {
    $('fileinfo').innerHTML = '';
    return;
  }
  const s = d.sizes;
  const mixed = s.some((p) => Math.abs(p[0] - s[0][0]) > 0.5 || Math.abs(p[1] - s[0][1]) > 0.5);
  $('fileinfo').innerHTML = `<b>${esc(d.name)}</b><br>${fmt(d.bytes.length / 1048576, 2)} MB · ${s.length} halaman · hal. 1: ${fmt(s[0][0])} × ${fmt(s[0][1])} mm${mixed ? ' · ukuran halaman campur' : ''}`;
}

async function openFile(f) {
  if (!f) return;
  if (!state.engine) return say('Mesin belum siap.', true);
  if (!/\.pdf$/i.test(f.name) && f.type !== 'application/pdf') return say('File harus PDF.', true);
  const bytes = new Uint8Array(await f.arrayBuffer());
  if (addDoc(f.name.replace(/\.pdf$/i, ''), bytes)) {
    state.view = 'layout';
    recompute();
  }
}

// ---- layout + preview ---------------------------------------------------------------------
let previewTimer = null;
let preview = null; // { key, bytes }

function recompute() {
  say('');
  state.layout = null;
  const d = activeDoc();
  if (state.engine && d) {
    try {
      state.layout = JSON.parse(state.engine.layout(JSON.stringify(d.sizes), JSON.stringify(job())));
    } catch (e) {
      say(String(e.message || e), true);
    }
  }
  if (state.layout) state.sheet = Math.min(state.sheet, state.layout.sheets.length - 1);
  else state.sheet = 0;
  draw();
}

function pageCount() {
  const d = activeDoc();
  if (state.view === 'doc') return d ? d.sizes.length : 0;
  return state.layout ? state.layout.sheets.length : 0;
}

function draw() {
  const d = activeDoc();
  const L = state.layout;
  $('views').querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.view === state.view)));
  const blocked = !!(d && d.pf && d.pf.overall === 'fail');
  $('ackwrap').hidden = !blocked;
  $('apply').disabled = !L || state.busy || (blocked && !$('ack').checked);
  $('download').disabled = !d || state.busy;
  $('saveas').hidden = !('showSaveFilePicker' in window);
  $('saveas').disabled = !d || state.busy;
  if (!d) {
    $('summary').innerHTML = '<span class="hint">Buka PDF untuk mulai.</span>';
    clearCanvas();
    navText();
    return;
  }
  if (state.view === 'doc') {
    $('summary').innerHTML = `<div class="big">${d.sizes.length}<small>halaman di tab ini</small></div>`;
  } else if (L) {
    const s = L.sheets[state.sheet];
    $('summary').innerHTML = `
      <div class="big">${L.sheet_count}<small>${state.mode === 'bleed' ? 'halaman' : 'sisi lembar'}</small></div>
      ${state.mode === 'bleed' ? '' : `<div class="big">${L.up_per_sheet}<small>up / lembar</small></div>`}
      <div class="hint">${fmt(s.size_mm[0])} × ${fmt(s.size_mm[1])} mm</div>`;
  } else {
    $('summary').innerHTML = '';
  }
  navText();
  if (state.view === 'layout') {
    if (L) drawOutline(L.sheets[state.sheet]);
    else clearCanvas();
  } else {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(drawPicture, 120);
  }
}

function navText() {
  const n = pageCount();
  const i = state.sheet;
  $('prev').disabled = i <= 0;
  $('next').disabled = i >= n - 1;
  if (!n) return ($('pos').textContent = '');
  if (state.view === 'doc') return ($('pos').textContent = `Halaman ${i + 1} dari ${n}`);
  const j = job();
  $('pos').textContent = `Lembar ${i + 1} dari ${n}${j.duplex || j.mode === 'booklet' ? (i % 2 ? ' · belakang' : ' · depan') : ''}`;
}

function clearCanvas() {
  const cv = $('cv');
  cv.getContext('2d').clearRect(0, 0, cv.width, cv.height);
}

function fitCanvas(W, H) {
  const cv = $('cv');
  cv.style.width = '';
  const k = Math.min(1100 / W, 760 / H);
  cv.width = Math.round(W * k);
  cv.height = Math.round(H * k);
  return k;
}

function drawOutline(s) {
  const [W, H] = s.size_mm;
  const k = fitCanvas(W, H);
  const cv = $('cv');
  const ctx = cv.getContext('2d');
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, cv.width, cv.height);
  const X = (x) => x * k, Y = (y) => cv.height - y * k;
  for (const c of s.cells) {
    const [x0, y0, x1, y1] = c.rect_mm;
    ctx.fillStyle = c.mirrored ? '#F6EFD0' : '#E8EEF6';
    ctx.fillRect(X(x0), Y(y1), (x1 - x0) * k, (y1 - y0) * k);
    ctx.strokeStyle = c.mirrored ? '#C9B65A' : '#004080';
    ctx.lineWidth = 1;
    ctx.strokeRect(X(x0) + 0.5, Y(y1) + 0.5, (x1 - x0) * k - 1, (y1 - y0) * k - 1);
    if (!c.mirrored) {
      const fs = Math.max(10, Math.min(48, Math.min(x1 - x0, y1 - y0) * k * 0.35));
      ctx.fillStyle = '#004080';
      ctx.font = `${fs}px Anton, sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(c.page + 1), X((x0 + x1) / 2), Y((y0 + y1) / 2));
    }
  }
  if (s.trim_mm) {
    const [x0, y0, x1, y1] = s.trim_mm;
    ctx.setLineDash([6, 4]);
    ctx.strokeStyle = '#F04000';
    ctx.strokeRect(X(x0), Y(y1), (x1 - x0) * k, (y1 - y0) * k);
    ctx.setLineDash([]);
  }
  ctx.strokeStyle = '#0F1B2D';
  ctx.lineWidth = 1;
  for (const [x0, y0, x1, y1] of s.marks_mm) {
    ctx.beginPath();
    ctx.moveTo(X(x0), Y(y0));
    ctx.lineTo(X(x1), Y(y1));
    ctx.stroke();
  }
}

/** The picture: the tab's own pages, or the pending layout made into a real PDF and drawn. */
async function drawPicture() {
  const d = activeDoc();
  if (!d) return;
  let bytes = d.bytes;
  if (state.view === 'picture') {
    if (!state.layout) return clearCanvas();
    const key = d.id + '|' + JSON.stringify(job());
    if (!preview || preview.key !== key) {
      try {
        preview = { key, bytes: state.engine.impose(d.bytes.slice(), JSON.stringify(job())) };
      } catch (e) {
        return say(String(e.message || e), true);
      }
    }
    bytes = preview.bytes;
  }
  const want = state.sheet;
  try {
    const doc = await pdfDoc(bytes);
    const page = await doc.getPage(want + 1);
    if (want !== state.sheet) return; // moved on meanwhile
    const vp1 = page.getViewport({ scale: 1 });
    const k = Math.min(1100 / vp1.width, 760 / vp1.height);
    const dpr = window.devicePixelRatio || 1;
    const vp = page.getViewport({ scale: k * dpr });
    const cv = $('cv');
    cv.width = Math.round(vp.width);
    cv.height = Math.round(vp.height);
    cv.style.width = Math.round(vp.width / dpr) + 'px';
    await page.render({ canvasContext: cv.getContext('2d'), viewport: vp }).promise;
  } catch (e) {
    say('Preview gambar gagal: ' + String(e.message || e), true);
  }
}

// ---- actions ------------------------------------------------------------------------------
async function apply() {
  const d = activeDoc();
  if (!d || !state.layout) return;
  state.busy = true;
  draw();
  say('Membuat layout…');
  await new Promise((r) => setTimeout(r, 20));
  try {
    const j = job();
    const t0 = performance.now();
    const out = state.engine.impose(d.bytes.slice(), JSON.stringify(j));
    const nd = addDoc(`${d.name} › ${jobLabel(j)}`, out, [...d.tags, MODE_TAG[j.mode]]);
    if (nd) {
      state.view = 'doc';
      state.sheet = 0;
      say(`Tab baru: ${fmt(out.length / 1048576, 2)} MB · ${fmt((performance.now() - t0) / 1000, 1)} detik. Bisa di-layout lagi atau disimpan.`, false, true);
    }
  } catch (e) {
    say(String(e.message || e), true);
  } finally {
    state.busy = false;
    recompute();
  }
}

function fileName(d) {
  const root = d.name.split(' › ')[0] || 'impose';
  return `${root}${d.tags.length ? '_' + d.tags.join('_') : ''}.pdf`;
}

async function save(saveAs) {
  const d = activeDoc();
  if (!d) return;
  const blob = new Blob([d.bytes], { type: 'application/pdf' });
  try {
    if (saveAs && 'showSaveFilePicker' in window) {
      const h = await window.showSaveFilePicker({ suggestedName: fileName(d), types: [{ description: 'PDF', accept: { 'application/pdf': ['.pdf'] } }] });
      const w = await h.createWritable();
      await w.write(blob);
      await w.close();
    } else {
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = fileName(d);
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 60000);
    }
    say(`Tersimpan: ${fileName(d)}`, false, true);
  } catch (e) {
    if (!(e && e.name === 'AbortError')) say(String(e.message || e), true);
  }
}

function say(t, err = false, ok = false) {
  const m = $('msg');
  m.textContent = t;
  m.className = 'msg' + (err ? ' err' : ok ? ' ok' : '');
}

// ---- wiring -------------------------------------------------------------------------------
$('file').addEventListener('change', (e) => { openFile(e.target.files[0]); e.target.value = ''; });
const drop = $('drop');
['dragenter', 'dragover'].forEach((t) => drop.addEventListener(t, (e) => { e.preventDefault(); drop.classList.add('over'); }));
['dragleave', 'drop'].forEach((t) => drop.addEventListener(t, (e) => { e.preventDefault(); drop.classList.remove('over'); }));
drop.addEventListener('drop', (e) => openFile(e.dataTransfer.files[0]));
$('modes').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-mode]');
  if (!b) return;
  state.mode = b.dataset.mode;
  $('modes').querySelectorAll('button').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
  if (state.view === 'doc') state.view = 'layout';
  renderOptions();
});
$('views').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-view]');
  if (!b) return;
  state.view = b.dataset.view;
  state.sheet = 0;
  draw();
});
$('tabs').addEventListener('click', (e) => {
  const x = e.target.closest('[data-close]');
  if (x) {
    const id = Number(x.dataset.close);
    state.docs = state.docs.filter((d) => d.id !== id);
    if (state.active === id) state.active = state.docs.length ? state.docs[state.docs.length - 1].id : null;
    renderTabs();
    showDocInfo();
    recompute();
    return;
  }
  const t = e.target.closest('[data-id]');
  if (t) {
    state.active = Number(t.dataset.id);
    state.sheet = 0;
    renderTabs();
    showDocInfo();
    recompute();
  }
});
$('prev').addEventListener('click', () => { state.sheet = Math.max(0, state.sheet - 1); draw(); });
$('next').addEventListener('click', () => { state.sheet = Math.min(pageCount() - 1, state.sheet + 1); draw(); });
$('apply').addEventListener('click', apply);
$('ack').addEventListener('change', draw);
$('pf').addEventListener('click', (e) => { if (e.target.closest('#do-outline')) outlineActive(); });
$('download').addEventListener('click', () => save(false));
$('saveas').addEventListener('click', () => save(true));

renderOptions();
loadEngine().then(draw);
