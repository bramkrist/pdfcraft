// INDIE Impose — impose.pixelindie.com. Everything runs in this browser: the PDF never leaves
// the computer. The engine is pdfcraft-impose compiled to WebAssembly (bramkrist/pdfcraft,
// apps/impose-web); the preview and the PDF come from the same layout.

const root = document.getElementById('app');
const user = root.dataset.user || '';

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

const state = {
  engine: null,
  file: null, // { name, bytes: Uint8Array }
  sizes: null, // [[w, h], ...] mm
  mode: 'repeat',
  layout: null,
  sheet: 0,
  busy: false,
};

root.innerHTML = `
<header class="top">
  <div class="brand"><div class="disc" aria-hidden="true">P</div><div><div class="word">IMPOSE</div></div></div>
  <div class="tag">PIXELINDIE · Pre-press</div>
  <div class="spacer"></div>
  <div class="who"><span id="ver"></span> · ${esc(user)}<a href="/logout.php">Keluar</a></div>
</header>
<div class="layout">
  <aside>
    <div id="nobanner" class="banner" hidden></div>
    <section class="panel">
      <h2>File</h2>
      <label class="drop" id="drop"><strong>Pilih atau tarik PDF ke sini</strong><span>Diproses di komputer ini, tidak di-upload.</span><input id="file" type="file" accept="application/pdf,.pdf"></label>
      <div class="fileinfo" id="fileinfo"></div>
    </section>
    <section class="panel">
      <h2>Mode</h2>
      <div class="seg" id="modes">${MODES.map(([k, t]) => `<button type="button" data-mode="${k}" aria-pressed="${k === state.mode}">${t}</button>`).join('')}</div>
    </section>
    <section class="panel" id="opts"></section>
  </aside>
  <section class="stage">
    <div class="summary" id="summary"></div>
    <div class="canvaswrap"><canvas id="cv" width="900" height="600"></canvas></div>
    <div class="nav"><button type="button" id="prev">‹</button><span id="pos"></span><button type="button" id="next">›</button></div>
    <div class="actions">
      <button type="button" class="btn" id="make" disabled>Buat PDF</button>
      <button type="button" class="btn ghost" id="saveas" disabled hidden>Simpan ke folder…</button>
    </div>
    <div class="msg" id="msg" role="status"></div>
  </section>
</div>`;

const $ = (id) => document.getElementById(id);

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
}
function fmt(n, d = 1) {
  return Number(n).toLocaleString('id-ID', { maximumFractionDigits: d });
}

// ---- options per mode -------------------------------------------------------------------
const num = (k, label, value, attrs = '') => `<div><label class="l" for="o-${k}">${label}</label><input type="number" id="o-${k}" data-k="${k}" value="${value}" step="any" ${attrs}></div>`;
const sel = (k, label, opts, value, cls = '') => `<div class="${cls}"><label class="l" for="o-${k}">${label}</label><select id="o-${k}" data-k="${k}">${opts.map(([v, t]) => `<option value="${v}" ${v === value ? 'selected' : ''}>${t}</option>`).join('')}</select></div>`;
const chk = (k, label, on) => `<label class="check full"><input type="checkbox" id="o-${k}" data-k="${k}" ${on ? 'checked' : ''}>${label}</label>`;

function sheetFields() {
  return `
    ${sel('sheet', 'Kertas', [...Object.keys(SHEETS).map((k) => [k, k]), ['custom', 'Custom']], 'A3+ 32×48', 'full')}
    <div class="full row2" id="customsheet" hidden>${num('sheet_w', 'Lebar (mm)', 320)}${num('sheet_h', 'Tinggi (mm)', 480)}</div>
    ${sel('orient', 'Arah', [['landscape', 'Landscape'], ['portrait', 'Portrait']], 'landscape')}
    ${num('margin', 'Margin (mm)', 5, 'min="0"')}`;
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
    ${chk('marks', 'Crop mark', true)}`;
}

function renderOptions() {
  let body = '';
  if (state.mode === 'repeat' || state.mode === 'sequential' || state.mode === 'cut-stack') {
    body = `<h2>Susunan</h2><div class="field">${sheetFields()}${gridFields(state.mode === 'cut-stack')}</div>
      ${state.mode === 'cut-stack' ? '<p class="hint">Tiap posisi jadi satu tumpukan yang urut dari lembar ke lembar. "Ulang blok" harus membagi jumlah kolom.</p>' : ''}`;
  } else if (state.mode === 'booklet') {
    body = `<h2>Booklet</h2><div class="field">
      ${sel('binding', 'Jilid', [['saddle', 'Steples tengah'], ['perfect', 'Lem panas (signature)'], ['continuous', 'Berurutan']], 'saddle', 'full')}
      ${num('signature', 'Halaman / signature', 16, 'min="4" step="4"')}
      ${sel('last', 'Signature terakhir', [['multiple-of-four', 'Kelipatan 4'], ['full', 'Penuh']], 'multiple-of-four')}
      ${sel('sheet', 'Kertas', [['auto', 'Otomatis (2× halaman)'], ...Object.keys(SHEETS).map((k) => [k, k]), ['custom', 'Custom']], 'auto', 'full')}
      <div class="full row2" id="customsheet" hidden>${num('sheet_w', 'Lebar (mm)', 480)}${num('sheet_h', 'Tinggi (mm)', 320)}</div>
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
  if (s === 'auto') return null;
  let [w, h] = custom ? [numOr('sheet_w', 320), numOr('sheet_h', 480)] : SHEETS[s];
  const wantLandscape = (val('orient') ?? 'landscape') === 'landscape' || state.mode === 'booklet';
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
  if (sheet) j.sheet_mm = sheet;
  if (state.mode === 'booklet') {
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
  j.margin_mm = numOr('margin', 5);
  const cols = numOr('cols', null), rows = numOr('rows', null);
  if (cols) j.cols = cols;
  if (rows) j.rows = rows;
  j.gutter_mm = [numOr('gx', 0), numOr('gy', 0)];
  j.bleed_mm = numOr('bleed', 0);
  if (val('duplex')) j.duplex = val('duplex');
  if (val('marks')) j.marks = true;
  if (state.mode === 'cut-stack') j.copies = numOr('copies', 1);
  return j;
}

// ---- engine -----------------------------------------------------------------------------
async function loadEngine() {
  try {
    const mod = await import('/a.php?f=pdfcraft_impose_web.js');
    await mod.default({ module_or_path: '/a.php?f=pdfcraft_impose_web_bg.wasm' });
    state.engine = mod;
    $('ver').textContent = 'engine ' + mod.version();
  } catch (e) {
    const b = $('nobanner');
    b.hidden = false;
    b.textContent = 'Mesin impose belum terpasang di server ini. Halaman siap, tunggu build WebAssembly selesai.';
    console.error(e);
  }
}

async function readFile(f) {
  if (!f) return;
  if (!/\.pdf$/i.test(f.name) && f.type !== 'application/pdf') return say('File harus PDF.', true);
  const bytes = new Uint8Array(await f.arrayBuffer());
  state.file = { name: f.name, bytes };
  state.sizes = null;
  if (state.engine) {
    try {
      state.sizes = JSON.parse(state.engine.pageSizes(bytes.slice()));
    } catch (e) {
      return say(String(e.message || e), true);
    }
  }
  const s = state.sizes;
  $('fileinfo').innerHTML = `<b>${esc(f.name)}</b><br>${fmt(bytes.length / 1048576, 2)} MB${s ? ` · ${s.length} halaman · hal. 1: ${fmt(s[0][0])} × ${fmt(s[0][1])} mm` : ''}${s && s.some((p) => Math.abs(p[0] - s[0][0]) > 0.5 || Math.abs(p[1] - s[0][1]) > 0.5) ? ' · ukuran halaman campur' : ''}`;
  recompute();
}

function recompute() {
  say('');
  state.layout = null;
  state.sheet = 0;
  if (state.engine && state.sizes) {
    try {
      state.layout = JSON.parse(state.engine.layout(JSON.stringify(state.sizes), JSON.stringify(job())));
    } catch (e) {
      say(String(e.message || e), true);
    }
  }
  draw();
}

function draw() {
  const L = state.layout;
  const cv = $('cv');
  const ctx = cv.getContext('2d');
  $('make').disabled = !L || state.busy;
  $('saveas').hidden = !('showSaveFilePicker' in window);
  $('saveas').disabled = !L || state.busy;
  if (!L) {
    $('summary').innerHTML = state.file ? '' : '<span class="hint">Pilih PDF untuk mulai.</span>';
    ctx.clearRect(0, 0, cv.width, cv.height);
    $('pos').textContent = '';
    $('prev').disabled = $('next').disabled = true;
    return;
  }
  const s = L.sheets[state.sheet];
  const [W, H] = s.size_mm;
  $('summary').innerHTML = `
    <div class="big">${L.sheet_count}<small>${state.mode === 'bleed' ? 'halaman' : 'sisi lembar'}</small></div>
    ${state.mode === 'bleed' ? '' : `<div class="big">${L.up_per_sheet}<small>up / lembar</small></div>`}
    <div class="hint">${fmt(W)} × ${fmt(H)} mm</div>`;
  const maxW = 1100, maxH = 760;
  const k = Math.min(maxW / W, maxH / H);
  cv.width = Math.round(W * k);
  cv.height = Math.round(H * k);
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
  $('pos').textContent = `Lembar ${state.sheet + 1} dari ${L.sheets.length}${L.sheets.length > 1 && state.mode !== 'bleed' && job().duplex ? (state.sheet % 2 ? ' · belakang' : ' · depan') : ''}`;
  $('prev').disabled = state.sheet === 0;
  $('next').disabled = state.sheet >= L.sheets.length - 1;
}

function outName() {
  const base = state.file.name.replace(/\.pdf$/i, '');
  const tag = { repeat: 'STEPREPEAT', sequential: 'NUP', 'cut-stack': 'UNIQUECOLLATE', booklet: 'BOOKLET', bleed: 'BLEED' }[state.mode];
  return `${base}_${tag}.pdf`;
}

async function make(saveAs) {
  if (!state.layout || !state.engine) return;
  state.busy = true;
  draw();
  say('Membuat PDF…');
  await new Promise((r) => setTimeout(r, 30));
  try {
    const t0 = performance.now();
    const out = state.engine.impose(state.file.bytes.slice(), JSON.stringify(job()));
    const blob = new Blob([out], { type: 'application/pdf' });
    if (saveAs && 'showSaveFilePicker' in window) {
      const h = await window.showSaveFilePicker({ suggestedName: outName(), types: [{ description: 'PDF', accept: { 'application/pdf': ['.pdf'] } }] });
      const w = await h.createWritable();
      await w.write(blob);
      await w.close();
    } else {
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = outName();
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 60000);
    }
    say(`Selesai: ${outName()} · ${fmt(out.length / 1048576, 2)} MB · ${fmt((performance.now() - t0) / 1000, 1)} detik`, false, true);
  } catch (e) {
    if (e && e.name === 'AbortError') say('');
    else say(String(e.message || e), true);
  } finally {
    state.busy = false;
    draw();
  }
}

function say(t, err = false, ok = false) {
  const m = $('msg');
  m.textContent = t;
  m.className = 'msg' + (err ? ' err' : ok ? ' ok' : '');
}

// ---- wiring -------------------------------------------------------------------------------
$('file').addEventListener('change', (e) => readFile(e.target.files[0]));
const drop = $('drop');
['dragenter', 'dragover'].forEach((t) => drop.addEventListener(t, (e) => { e.preventDefault(); drop.classList.add('over'); }));
['dragleave', 'drop'].forEach((t) => drop.addEventListener(t, (e) => { e.preventDefault(); drop.classList.remove('over'); }));
drop.addEventListener('drop', (e) => readFile(e.dataTransfer.files[0]));
$('modes').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-mode]');
  if (!b) return;
  state.mode = b.dataset.mode;
  $('modes').querySelectorAll('button').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
  renderOptions();
});
$('prev').addEventListener('click', () => { state.sheet = Math.max(0, state.sheet - 1); draw(); });
$('next').addEventListener('click', () => { state.sheet = Math.min(state.layout.sheets.length - 1, state.sheet + 1); draw(); });
$('make').addEventListener('click', () => make(false));
$('saveas').addEventListener('click', () => make(true));

renderOptions();
loadEngine().then(() => { if (state.file) readFile(new File([state.file.bytes], state.file.name)); });
