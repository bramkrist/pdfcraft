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
  ['manual', 'Manual'],
];
const MODE_TAG = { repeat: 'STEPREPEAT', sequential: 'NUP', 'cut-stack': 'UNIQUECOLLATE', booklet: 'BOOKLET', bleed: 'BLEED', manual: 'MANUAL' };

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
  sel: null, // manual: index of the selected page on the current sheet
  drag: null, // manual: { i, dx, dy, guides }
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
        <button type="button" data-view="pages">Edit halaman</button>
        <button type="button" data-view="layout">Layout garis</button>
        <button type="button" data-view="picture">Layout gambar</button>
      </div>
    </div>
    <div class="canvaswrap" id="cvwrap"><canvas id="cv" width="900" height="600"></canvas></div>
    <div class="org" id="org" hidden>
      <div class="orgbar">
        <button type="button" class="btn ghost small" data-o="rotl" title="Putar kiri (Shift+R)">⟲</button>
        <button type="button" class="btn ghost small" data-o="rotr" title="Putar kanan (R)">⟳</button>
        <button type="button" class="btn ghost small" data-o="dup" title="Duplikat (Ctrl+D)">Duplikat</button>
        <button type="button" class="btn ghost small" data-o="del" title="Hapus (Del)">Hapus</button>
        <button type="button" class="btn ghost small" data-o="left" title="Geser ke kiri">◀</button>
        <button type="button" class="btn ghost small" data-o="right" title="Geser ke kanan">▶</button>
        <button type="button" class="btn ghost small" data-o="blank" title="Sisipkan halaman kosong sesudah pilihan">+ Kosong</button>
        <button type="button" class="btn ghost small" data-o="undo" title="Batalkan (Ctrl+Z)">↶ Batal</button>
        <button type="button" class="btn ghost small" data-o="reset">Ulang</button>
      </div>
      <div class="orgbar">
        <label class="l" for="o-from" style="margin:0">Sisipkan dari tab</label>
        <select id="o-from"></select>
        <input type="text" id="o-range" placeholder="semua / 1-3,5" style="width:140px">
        <button type="button" class="btn ghost small" data-o="insert">Sisipkan</button>
        <span class="hint" id="orginfo"></span>
      </div>
      <div class="orggrid" id="orggrid" tabindex="0" aria-label="Halaman"></div>
      <p class="hint">Klik pilih · <kbd>Ctrl</kbd>/<kbd>Shift</kbd>+klik pilih banyak · tarik untuk pindah · <kbd>R</kbd> putar · <kbd>Del</kbd> hapus · <kbd>Ctrl+C</kbd>/<kbd>Ctrl+V</kbd> salin · <kbd>Ctrl+Z</kbd> batal. Begitu pindah ke Layout (atau klik Terapkan), perubahan jadi tab baru dan layout memakai urutan baru itu. Link, komentar & bookmark ikut halamannya.</p>
    </div>
    <div class="nav" id="navbar"><button type="button" id="prev" aria-label="Sebelumnya">‹</button><span id="pos"></span><button type="button" id="next" aria-label="Berikutnya">›</button></div>
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
  } else if (state.mode === 'manual') {
    const d = activeDoc();
    const pages = d ? d.sizes.map((_, i) => [String(i), `Hal. ${i + 1}`]) : [['0', 'Hal. 1']];
    body = `<h2>Manual</h2><div class="field">
      ${sel('sheet', 'Kertas', [...Object.keys(SHEETS).map((k) => [k, k]), ['custom', 'Custom']], 'A3+ 32×48', 'full')}
      <div class="full row2" id="customsheet" hidden>${num('sheet_w', 'Lebar (mm)', 320)}${num('sheet_h', 'Tinggi (mm)', 480)}</div>
      ${sel('orient', 'Arah', [['landscape', 'Landscape'], ['portrait', 'Portrait']], 'landscape')}
      ${num('bleed', 'Bleed di file (mm)', 0, 'min="0"')}
      ${chk('snap', 'Snap ke tepi halaman lain & kertas', true)}
    </div>
    <div class="mbar" style="margin-top:12px">
      <button type="button" class="btn ghost small" data-m="seed">Isi dari Step &amp; Repeat</button>
      <button type="button" class="btn ghost small" data-m="clear">Kosongkan lembar</button>
    </div>
    <div class="field" style="margin-top:12px">
      ${sel('mpage', 'Halaman', pages, '0')}
      <div><label class="l">&nbsp;</label><button type="button" class="btn ghost small" data-m="add" style="width:100%">+ Tambah ke lembar</button></div>
    </div>
    <div class="mbar" style="margin-top:8px">
      <button type="button" class="btn ghost small" data-m="sheet-add">+ Lembar</button>
      <button type="button" class="btn ghost small" data-m="sheet-dup">Duplikat lembar</button>
      <button type="button" class="btn ghost small" data-m="sheet-del">Hapus lembar</button>
    </div>
    <div class="msel" id="msel" hidden></div>
    <div class="mstat" id="mstat"></div>
    <p class="hint">Tarik halaman di layout garis. <kbd>←↑→↓</kbd> geser 0,5 mm (<kbd>Shift</kbd> 5 mm) · <kbd>R</kbd> putar · <kbd>D</kbd> duplikat · <kbd>Del</kbd> hapus.</p>
    ${markFields()}`;
  } else if (state.mode === 'bleed') {
    body = `<h2>Bleed</h2><div class="field">
      ${num('add', 'Bleed (mm)', 3, 'min="0"')}
      ${sel('method', 'Cara', [['mirror', 'Cermin tepi'], ['scale', 'Perbesar halaman']], 'mirror')}
    </div><p class="hint">Tepi halaman sekarang dianggap garis potong. Hasilnya punya TrimBox dan BleedBox.</p>`;
  }
  $('opts').innerHTML = body;
  if (state.mode === 'manual') manualFieldsFromDoc();
  const fb = fileBleed(activeDoc());
  const bleedInput = document.querySelector('[data-k="bleed"]');
  if (fb !== null && bleedInput) bleedInput.value = fb;
  $('opts').querySelectorAll('[data-k]').forEach((el) => el.addEventListener('input', () => {
    if (state.mode === 'manual' && !['mpage', 'snap'].includes(el.dataset.k)) manualSheetFromFields();
    recompute();
  }));
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
  if (state.mode === 'manual') {
    const d = activeDoc();
    const m = d ? manualOf(d) : { sheet_mm: sheet || [480, 320], sheets: [[]] };
    j.sheet_mm = m.sheet_mm;
    j.bleed_mm = numOr('bleed', 0);
    j.manual = m.sheets.map((side) => side.map((c) => ({ page: c.page, x_mm: round2(c.x), y_mm: round2(c.y), turn: c.turn })));
    j.marks = marksJob();
    return j;
  }
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
  const marks = marksJob();
  if (marks) j.marks = marks;
  if (state.mode === 'cut-stack') j.copies = numOr('copies', 1);
  return j;
}

function marksJob() {
  if (!val('marks')) return undefined;
  return {
    length_mm: numOr('mlen', 5),
    offset_mm: numOr('moff', 3),
    style: val('mstyle'),
    width_pt: numOr('mwidth', 0.5),
    registration: val('mcolour') === 'registration',
  };
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
  const names = r.inks.spots.map((s) => s.name);
  const spots = r.inks.spots
    .map((s, i) => {
      const finishing = FINISHING.test(s.name);
      const others = names.filter((n) => n !== s.name).map((n) => `<option value="alias:${esc(n)}">Gabung ke ${esc(n)}</option>`).join('');
      return `<li class="spotrow"><span class="ink spot on"></span><span class="spotname">${esc(s.name)} <span class="hint">hal. ${s.pages.join(', ')}${finishing ? ' · layer produksi' : ''}</span></span>
        <select class="inkact" data-i="${i}" aria-label="Tindakan untuk ${esc(s.name)}"><option value="keep">Biarkan (plat sendiri)</option>${finishing ? '' : '<option value="process">Jadikan CMYK</option>'}${others}</select></li>`;
    })
    .join('');
  const rows = pf.checks.map((c) => `<li class="pf-${c.status}"><span class="dot"></span><div><b>${esc(c.title)}</b> <span class="st">${STATUS_TEXT[c.status] || c.status}</span><br><span class="pfmsg">${esc(c.message)}</span></div></li>`).join('');
  const fonts = r.fonts.map((f) => `<li>${f.embedded ? '' : '<b class="bad">TIDAK TERTANAM</b> '}${esc(f.name)} <span class="hint">${esc(f.subtype)}${f.subset ? ' · subset' : ''}</span></li>`).join('');
  const imgs = r.images.slice(0, 50).map((i) => `<li class="${i.missing || i.ppi < 200 ? 'bad' : i.ppi < 300 ? 'meh' : ''}">hal. ${i.page} · ${i.missing ? esc(i.missing) : `${Math.round(i.ppi)} DPI · ${i.width_px}×${i.height_px} px · ${fmt(i.size_mm[0], 0)}×${fmt(i.size_mm[1], 0)} mm · ${esc(i.colour)}${i.tac ? ` · TAC ${Math.round(i.tac.max)}%${i.tac.over > 0.001 ? ` (${fmt(i.tac.over * 100, 1)}% piksel di atas batas)` : ''}` : ''}`}</li>`).join('');
  $('pf').innerHTML = `
    <div class="pfhead pf-${pf.overall}"><span class="dot"></span>${pf.overall === 'pass' ? 'Siap cetak' : pf.overall === 'warn' ? 'Ada yang perlu dicek' : 'Ada masalah'}</div>
    <div class="inks">${ink(r.inks.cyan, 'Cyan', 'c')}${ink(r.inks.magenta, 'Magenta', 'm')}${ink(r.inks.yellow, 'Yellow', 'y')}${ink(r.inks.black, 'Black', 'k')}
      ${r.inks.rgb ? '<span class="tagchip warn">RGB</span>' : ''}${r.inks.registration ? '<span class="tagchip warn">Registration</span>' : ''}${r.inks.lab ? '<span class="tagchip warn">Lab</span>' : ''}</div>
    ${spots ? `<div class="inkman"><b>Ink manager</b><ul class="plain spots">${spots}</ul><div class="fix"><button type="button" class="btn ghost small" id="do-inks" disabled>Terapkan tinta</button><span class="hint">Jadi tab baru. "Jadikan CMYK" memakai warna alternatif dari file itu sendiri (yang tampil di layar). "Gabung" menyatukan dua nama spot jadi satu plat. Gambar/gradasi dalam spot dibiarkan dan dilaporkan.</span></div></div>` : ''}
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

const FINISHING = /cut|dieline|die line|foil|varnish|spot ?uv|white|emboss|crease|perfor/i;

/** The ink actions chosen in the panel, as the engine wants them. */
function inkActions() {
  const d = activeDoc();
  const spots = d && d.pf ? d.pf.report.inks.spots : [];
  const out = {};
  document.querySelectorAll('.inkact').forEach((sel) => {
    const s = spots[Number(sel.dataset.i)];
    if (!s || sel.value === 'keep') return;
    out[s.name] = sel.value === 'process' ? { action: 'to-process' } : { action: 'alias', into: sel.value.slice(6) };
  });
  return out;
}

async function inksActive() {
  const d = activeDoc();
  const actions = inkActions();
  if (!d || !state.engine || !Object.keys(actions).length) return;
  state.busy = true;
  draw();
  say('Mengubah tinta…');
  await new Promise((r) => setTimeout(r, 20));
  let note = null;
  try {
    const res = state.engine.manageInks(d.bytes.slice(), JSON.stringify(actions));
    const rep = JSON.parse(res.report);
    const done = [
      ...Object.entries(rep.converted).map(([n, c]) => `${n} → CMYK (${c}×)`),
      ...Object.entries(rep.aliased).map(([n]) => `${n} → ${actions[n] && actions[n].into}`),
    ];
    if (!done.length) {
      note = [rep.kept.length ? `Tidak ada yang bisa diubah: ${rep.kept.slice(0, 4).join('; ')}` : 'Tidak ada yang berubah.', true];
      return;
    }
    const nd = addDoc(`${d.name} › Tinta`, res.bytes, [...d.tags, 'INK']);
    if (nd) {
      state.view = 'doc';
      state.sheet = 0;
      const kept = rep.kept.length ? ` Dibiarkan: ${[...new Set(rep.kept)].slice(0, 4).join('; ')}.` : '';
      note = [`Tinta diubah: ${done.join(', ')}.${kept}`, !!rep.kept.length, !rep.kept.length];
    }
  } catch (e) {
    note = ['Ubah tinta gagal: ' + String(e.message || e), true];
  } finally {
    state.busy = false;
    recompute();
    if (note) say(...note);
  }
}

async function outlineActive() {
  const d = activeDoc();
  if (!d || !state.engine) return;
  state.busy = true;
  draw();
  say('Mengubah teks jadi outline…');
  await new Promise((r) => setTimeout(r, 20));
  let note = null;
  try {
    const t0 = performance.now();
    const res = state.engine.outlineText(d.bytes.slice());
    const rep = JSON.parse(res.report);
    const bytes = res.bytes;
    if (!rep.outlined) {
      note = [rep.kept.length ? `Tidak ada teks yang bisa di-outline: ${rep.kept.map((k) => `hal. ${k.page} ${k.reason}`).slice(0, 4).join('; ')}` : 'Tidak ada teks untuk di-outline.', !!rep.kept.length];
      return;
    }
    const nd = addDoc(`${d.name} › Outline`, bytes, [...d.tags, 'OUTLINE']);
    if (nd) {
      state.view = 'doc';
      state.sheet = 0;
      const kept = rep.kept.length ? ` ${rep.kept.length} bagian dibiarkan sebagai teks: ${[...new Set(rep.kept.map((k) => `hal. ${k.page} (${k.reason})`))].slice(0, 4).join(', ')}.` : '';
      note = [`Outline selesai: ${rep.outlined} blok teks, ${rep.glyphs} huruf, ${fmt((performance.now() - t0) / 1000, 1)} detik.${kept}`, !!rep.kept.length, !rep.kept.length];
    }
  } catch (e) {
    note = ['Outline gagal: ' + String(e.message || e), true];
  } finally {
    state.busy = false;
    recompute();
    if (note) say(...note);
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
  if (state.mode === 'manual' && d) state.sheet = Math.min(state.sheet, manualOf(d).sheets.length - 1);
  else if (state.layout) state.sheet = Math.min(state.sheet, state.layout.sheets.length - 1);
  else state.sheet = 0;
  draw();
}

function pageCount() {
  const d = activeDoc();
  if (state.view === 'doc') return d ? d.sizes.length : 0;
  if (state.mode === 'manual' && d) return manualOf(d).sheets.length;
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
  const pagesView = state.view === 'pages';
  $('org').hidden = !pagesView;
  $('cvwrap').hidden = pagesView;
  $('navbar').hidden = pagesView;
  if (pagesView) {
    $('apply').disabled = state.busy || !orgOf(d).items.length;
    $('summary').innerHTML = `<div class="big">${orgOf(d).items.length}<small>halaman setelah diedit</small></div><div class="hint">dari ${d.sizes.length} halaman di tab ini</div>`;
    renderOrg();
    return;
  }
  if (state.view === 'doc') {
    $('summary').innerHTML = `<div class="big">${d.sizes.length}<small>halaman di tab ini</small></div>`;
  } else if (state.mode === 'manual') {
    const m = manualOf(d);
    $('summary').innerHTML = `
      <div class="big">${m.sheets.length}<small>sisi lembar</small></div>
      <div class="big">${curSide(d).length}<small>halaman di lembar ini</small></div>
      <div class="hint">${fmt(m.sheet_mm[0])} × ${fmt(m.sheet_mm[1])} mm</div>`;
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
  const cv = $('cv');
  cv.classList.toggle('manual', state.mode === 'manual' && state.view === 'layout');
  if (state.mode === 'manual') renderManualSel();
  if (state.view === 'layout' && state.mode === 'manual') {
    drawManual();
  } else if (state.view === 'layout') {
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

// ---- manual imposition --------------------------------------------------------------------
// The layout lives in the tab (d.manual): sheets of { page, x, y, turn } where x, y is the
// lower-left corner of the page's footprint (bleed included) in mm from the sheet's lower left,
// and turn is degrees anticlockwise. The engine draws the real PDF and the marks from it.

const round2 = (v) => Math.round(v * 100) / 100;

function manualOf(d) {
  if (!d.manual) d.manual = { sheet_mm: sheetMm() || [480, 320], sheets: [[]] };
  if (!d.manual.sheets.length) d.manual.sheets.push([]);
  return d.manual;
}

/** Width and height of a placed page's footprint (turned). */
function footprint(d, c) {
  const [w, h] = d.sizes[c.page] || [10, 10];
  return c.turn % 180 === 0 ? [w, h] : [h, w];
}

function rectOf(d, c) {
  const [fw, fh] = footprint(d, c);
  return [c.x, c.y, c.x + fw, c.y + fh];
}

function manualFromLayout(d, L) {
  const sheets = L.sheets.map((s) => s.cells.filter((c) => !c.mirrored).map((c) => {
    const turn = ((c.turn % 360) + 360) % 360;
    const probe = { page: c.page, x: 0, y: 0, turn };
    const [fw, fh] = footprint(d, probe);
    // The engine centres a page in its cell; the visible rect has the same centre.
    const cx = (c.rect_mm[0] + c.rect_mm[2]) / 2, cy = (c.rect_mm[1] + c.rect_mm[3]) / 2;
    return { page: c.page, x: round2(cx - fw / 2), y: round2(cy - fh / 2), turn };
  }));
  return { sheet_mm: L.sheets[0] ? L.sheets[0].size_mm.slice() : [480, 320], sheets: sheets.length ? sheets : [[]] };
}

/** Put the tab's manual sheet size into the fields. */
function manualFieldsFromDoc() {
  const d = activeDoc();
  if (!d) return;
  const [w, h] = manualOf(d).sheet_mm;
  const preset = Object.entries(SHEETS).find(([, v]) => (v[0] === w && v[1] === h) || (v[0] === h && v[1] === w));
  const set = (k, v) => { const el = document.querySelector(`[data-k="${k}"]`); if (el) el.value = v; };
  if (preset) set('sheet', preset[0]);
  else { set('sheet', 'custom'); set('sheet_w', Math.min(w, h)); set('sheet_h', Math.max(w, h)); }
  set('orient', w >= h ? 'landscape' : 'portrait');
  sheetMm();
}

function manualSheetFromFields() {
  const d = activeDoc();
  const s = sheetMm();
  if (d && s) manualOf(d).sheet_mm = s;
}

function curSide(d) {
  const m = manualOf(d);
  return m.sheets[Math.min(state.sheet, m.sheets.length - 1)];
}

function overlaps(a, b) {
  return a[0] < b[2] - 0.01 && b[0] < a[2] - 0.01 && a[1] < b[3] - 0.01 && b[1] < a[3] - 0.01;
}

/** What is wrong on the current sheet: trims over each other, bleed over another trim, off the sheet. */
function manualProblems(d) {
  const side = curSide(d);
  const b = numOr('bleed', 0);
  const [W, H] = manualOf(d).sheet_mm;
  const rects = side.map((c) => rectOf(d, c));
  const trims = rects.map((r) => [r[0] + b, r[1] + b, r[2] - b, r[3] - b]);
  const bad = new Set(), warn = new Set();
  let off = 0;
  rects.forEach((r, i) => {
    if (r[0] < -0.01 || r[1] < -0.01 || r[2] > W + 0.01 || r[3] > H + 0.01) { warn.add(i); off++; }
    trims.forEach((t, j) => {
      if (i === j) return;
      if (overlaps(trims[i], t)) { bad.add(i); bad.add(j); }
      else if (overlaps(r, t)) warn.add(i);
    });
  });
  return { bad, warn, off };
}

/** Free spot for a page of footprint fw x fh: next to existing pages, else the lower left. */
function freeSpot(d, fw, fh) {
  const side = curSide(d);
  const [W, H] = manualOf(d).sheet_mm;
  const rects = side.map((c) => rectOf(d, c));
  const cands = [[5, H - 5 - fh]];
  for (const r of rects) cands.push([r[2], r[1]], [r[0], r[1] - fh], [r[0], r[3]], [r[0] - fw, r[1]]);
  cands.sort((a, b) => (b[1] - a[1]) || (a[0] - b[0]));
  for (const [x, y] of cands) {
    const r = [x, y, x + fw, y + fh];
    if (r[0] >= -0.01 && r[1] >= -0.01 && r[2] <= W + 0.01 && r[3] <= H + 0.01 && !rects.some((o) => overlaps(o, r))) return [round2(x), round2(y)];
  }
  return [round2((W - fw) / 2), round2((H - fh) / 2)];
}

function manualChanged() {
  // Marks and the summary come from the engine; keep any message the user should still see.
  const d = activeDoc();
  state.layout = null;
  if (state.engine && d) {
    try {
      state.layout = JSON.parse(state.engine.layout(JSON.stringify(d.sizes), JSON.stringify(job())));
    } catch (e) {
      state.layout = null;
    }
  }
  draw();
}

function manualAction(what) {
  const d = activeDoc();
  if (!d) return;
  const m = manualOf(d);
  const side = curSide(d);
  if (what === 'seed') {
    const j = { mode: 'repeat', sheet_mm: m.sheet_mm, margin_mm: 5, bleed_mm: numOr('bleed', 0), rotate: true };
    try {
      const L = JSON.parse(state.engine.layout(JSON.stringify(d.sizes), JSON.stringify(j)));
      const filled = manualFromLayout(d, L);
      m.sheets[state.sheet] = filled.sheets[0] || [];
    } catch (e) {
      return say('Tidak bisa isi otomatis: ' + String(e.message || e), true);
    }
  } else if (what === 'clear') {
    m.sheets[state.sheet] = [];
    state.sel = null;
  } else if (what === 'add') {
    const page = Math.max(0, Math.min(d.sizes.length - 1, Number(val('mpage')) || 0));
    const c = { page, x: 0, y: 0, turn: 0 };
    const [fw, fh] = footprint(d, c);
    [c.x, c.y] = freeSpot(d, fw, fh);
    side.push(c);
    state.sel = side.length - 1;
  } else if (what === 'sheet-add') {
    m.sheets.splice(state.sheet + 1, 0, []);
    state.sheet++;
    state.sel = null;
  } else if (what === 'sheet-dup') {
    m.sheets.splice(state.sheet + 1, 0, side.map((c) => ({ ...c })));
    state.sheet++;
  } else if (what === 'sheet-del') {
    if (m.sheets.length === 1) m.sheets[0] = [];
    else m.sheets.splice(state.sheet, 1);
    state.sheet = Math.max(0, Math.min(state.sheet, m.sheets.length - 1));
    state.sel = null;
  } else if (state.sel !== null && side[state.sel]) {
    const c = side[state.sel];
    if (what === 'rotate') {
      const r = rectOf(d, c);
      const cx = (r[0] + r[2]) / 2, cy = (r[1] + r[3]) / 2;
      c.turn = (c.turn + 90) % 360;
      const [fw, fh] = footprint(d, c);
      c.x = round2(cx - fw / 2);
      c.y = round2(cy - fh / 2);
    } else if (what === 'dup') {
      const n = { ...c };
      const [fw, fh] = footprint(d, n);
      [n.x, n.y] = freeSpot(d, fw, fh);
      side.push(n);
      state.sel = side.length - 1;
    } else if (what === 'del') {
      side.splice(state.sel, 1);
      state.sel = null;
    }
  }
  manualChanged();
}

function renderManualSel() {
  const box = $('msel'), stat = $('mstat');
  const d = activeDoc();
  if (!box || !d) return;
  const pageSel = document.querySelector('[data-k="mpage"]');
  if (pageSel && pageSel.options.length !== d.sizes.length) {
    pageSel.innerHTML = d.sizes.map((_, i) => `<option value="${i}">Hal. ${i + 1}</option>`).join('');
  }
  const side = curSide(d);
  const c = state.sel !== null ? side[state.sel] : null;
  if (!c) {
    box.hidden = true;
  } else {
    const b = numOr('bleed', 0);
    const [, H] = manualOf(d).sheet_mm;
    const [, fh] = footprint(d, c);
    // Shown as the trim's distance from the sheet's left and top edges, as a ruler reads it.
    const left = round2(c.x + b), top = round2(H - (c.y + fh) + b);
    if (!box.contains(document.activeElement)) {
      box.hidden = false;
      box.innerHTML = `<div class="field">
        <div class="full"><b>Hal. ${c.page + 1}</b> <span class="hint">· putar ${c.turn}° · ${fmt(footprint(d, c)[0])} × ${fmt(fh)} mm</span></div>
        <div><label class="l" for="m-left">Dari kiri (potong, mm)</label><input type="number" id="m-left" step="0.1" value="${left}"></div>
        <div><label class="l" for="m-top">Dari atas (potong, mm)</label><input type="number" id="m-top" step="0.1" value="${top}"></div>
      </div>
      <div class="mbar" style="margin-top:8px">
        <button type="button" class="btn ghost small" data-m="rotate">⟲ Putar 90°</button>
        <button type="button" class="btn ghost small" data-m="dup">Duplikat</button>
        <button type="button" class="btn ghost small" data-m="del">Hapus</button>
      </div>`;
    }
  }
  const p = manualProblems(d);
  const n = side.length;
  const err = !state.layout && n ? ' Layout belum valid untuk mesin.' : '';
  if (p.bad.size) {
    stat.className = 'mstat err';
    stat.textContent = `${p.bad.size} halaman area potongnya saling tumpuk (merah).${err}`;
  } else if (p.warn.size) {
    stat.className = 'mstat warn';
    stat.textContent = `${p.warn.size} halaman: bleed menutupi area potong halaman lain atau keluar kertas (kuning).${p.off ? ' Yang keluar kertas terpotong di tepi.' : ''}${err}`;
  } else {
    stat.className = 'mstat';
    stat.textContent = n ? `${n} halaman di lembar ini, tidak ada yang tumpuk.${err}` : 'Lembar kosong. Tambah halaman atau isi dari Step & Repeat.';
  }
}

function drawManual() {
  const d = activeDoc();
  if (!d) return clearCanvas();
  const m = manualOf(d);
  const [W, H] = m.sheet_mm;
  const k = fitCanvas(W, H);
  const cv = $('cv');
  const ctx = cv.getContext('2d');
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, cv.width, cv.height);
  const X = (x) => x * k, Y = (y) => cv.height - y * k;
  const b = numOr('bleed', 0);
  const side = curSide(d);
  const p = manualProblems(d);
  side.forEach((c, i) => {
    const [x0, y0, x1, y1] = rectOf(d, c);
    ctx.fillStyle = p.bad.has(i) ? 'rgba(180,35,24,.22)' : p.warn.has(i) ? 'rgba(240,208,80,.35)' : '#E8EEF6';
    ctx.fillRect(X(x0), Y(y1), (x1 - x0) * k, (y1 - y0) * k);
    if (b > 0) {
      ctx.setLineDash([3, 3]);
      ctx.strokeStyle = '#7A8899';
      ctx.strokeRect(X(x0) + 0.5, Y(y1) + 0.5, (x1 - x0) * k - 1, (y1 - y0) * k - 1);
      ctx.setLineDash([]);
    }
    ctx.strokeStyle = i === state.sel ? '#F04000' : '#004080';
    ctx.lineWidth = i === state.sel ? 2.5 : 1;
    ctx.strokeRect(X(x0 + b), Y(y1 - b), (x1 - x0 - 2 * b) * k, (y1 - y0 - 2 * b) * k);
    ctx.lineWidth = 1;
    // Page number, turned as the page is, with a bar along the page's head.
    ctx.save();
    ctx.translate(X((x0 + x1) / 2), Y((y0 + y1) / 2));
    ctx.rotate((-c.turn * Math.PI) / 180);
    const [pw, ph] = d.sizes[c.page] || [10, 10];
    const fs = Math.max(10, Math.min(48, Math.min(pw, ph) * k * 0.35));
    ctx.fillStyle = '#004080';
    ctx.font = `${fs}px Anton, sans-serif`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(c.page + 1), 0, 0);
    ctx.fillRect((-pw / 2 + b) * k + 4, (-ph / 2 + b) * k + 4, Math.max(4, (pw - 2 * b) * k - 8), 3);
    ctx.restore();
  });
  // Marks from the engine when the layout is valid.
  const s = state.layout && state.layout.sheets[state.sheet];
  if (s) {
    ctx.strokeStyle = '#0F1B2D';
    for (const [x0, y0, x1, y1] of s.marks_mm) {
      ctx.beginPath();
      ctx.moveTo(X(x0), Y(y0));
      ctx.lineTo(X(x1), Y(y1));
      ctx.stroke();
    }
  }
  if (state.drag && state.drag.guides) {
    ctx.strokeStyle = '#F04000';
    ctx.setLineDash([5, 4]);
    for (const g of state.drag.guides) {
      ctx.beginPath();
      if (g.x !== undefined) { ctx.moveTo(X(g.x), 0); ctx.lineTo(X(g.x), cv.height); }
      else { ctx.moveTo(0, Y(g.y)); ctx.lineTo(cv.width, Y(g.y)); }
      ctx.stroke();
    }
    ctx.setLineDash([]);
  }
}

/** Pointer position in sheet millimetres (the canvas may be letterboxed by CSS). */
function toMm(e) {
  const d = activeDoc();
  const [W, H] = manualOf(d).sheet_mm;
  const cv = $('cv');
  const r = cv.getBoundingClientRect();
  const sc = Math.min(r.width / cv.width, r.height / cv.height);
  const ox = r.left + (r.width - cv.width * sc) / 2, oy = r.top + (r.height - cv.height * sc) / 2;
  const px = (e.clientX - ox) / sc, py = (e.clientY - oy) / sc;
  return { x: (px / cv.width) * W, y: H - (py / cv.height) * H, perPx: W / (cv.width * sc) };
}

/** Snap a moving rect's x (or y) so one of its edges meets an edge on the sheet. */
function snapAxis(pos, size, b, targets, tol) {
  const mine = [0, size, b, size - b, size / 2];
  let best = null;
  for (const m of mine) {
    for (const t of targets) {
      const delta = t - (pos + m);
      if (Math.abs(delta) <= tol && (!best || Math.abs(delta) < Math.abs(best.delta))) best = { delta, line: t };
    }
  }
  return best;
}

function hitCell(d, pt) {
  const side = curSide(d);
  for (let i = side.length - 1; i >= 0; i--) {
    const r = rectOf(d, side[i]);
    if (pt.x >= r[0] && pt.x <= r[2] && pt.y >= r[1] && pt.y <= r[3]) return i;
  }
  return null;
}

function manualPointerDown(e) {
  if (state.mode !== 'manual' || state.view !== 'layout') return;
  const d = activeDoc();
  if (!d) return;
  const pt = toMm(e);
  const i = hitCell(d, pt);
  state.sel = i;
  if (i !== null) {
    const c = curSide(d)[i];
    state.drag = { i, dx: pt.x - c.x, dy: pt.y - c.y, guides: [] };
    $('cv').setPointerCapture(e.pointerId);
    $('cv').classList.add('dragging');
  }
  draw();
}

function manualPointerMove(e) {
  if (state.mode !== 'manual' || state.view !== 'layout') return;
  const d = activeDoc();
  if (!d) return;
  const pt = toMm(e);
  if (!state.drag) {
    $('cv').classList.toggle('grab', hitCell(d, pt) !== null);
    return;
  }
  const side = curSide(d);
  const c = side[state.drag.i];
  if (!c) return;
  let x = pt.x - state.drag.dx, y = pt.y - state.drag.dy;
  const [fw, fh] = footprint(d, c);
  const guides = [];
  if (val('snap') && !e.altKey) {
    const b = numOr('bleed', 0);
    const [W, H] = manualOf(d).sheet_mm;
    const tx = [0, W, W / 2], ty = [0, H, H / 2];
    side.forEach((o, j) => {
      if (j === state.drag.i) return;
      const r = rectOf(d, o);
      tx.push(r[0], r[2], r[0] + b, r[2] - b);
      ty.push(r[1], r[3], r[1] + b, r[3] - b);
    });
    const tol = 8 * pt.perPx;
    const sx = snapAxis(x, fw, b, tx, tol), sy = snapAxis(y, fh, b, ty, tol);
    if (sx) { x += sx.delta; guides.push({ x: sx.line }); }
    if (sy) { y += sy.delta; guides.push({ y: sy.line }); }
  }
  c.x = round2(guides.some((g) => g.x !== undefined) ? x : Math.round(x * 10) / 10);
  c.y = round2(guides.some((g) => g.y !== undefined) ? y : Math.round(y * 10) / 10);
  state.drag.guides = guides;
  draw();
}

function manualPointerUp() {
  if (!state.drag) return;
  state.drag = null;
  $('cv').classList.remove('dragging');
  manualChanged();
}

function manualKey(e) {
  if (state.mode !== 'manual' || state.view !== 'layout' || state.sel === null) return;
  const t = e.target;
  if (t && (t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.tagName === 'TEXTAREA')) return;
  const d = activeDoc();
  if (!d) return;
  const c = curSide(d)[state.sel];
  if (!c) return;
  const step = e.shiftKey ? 5 : 0.5;
  const moves = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
  if (moves[e.key]) {
    c.x = round2(c.x + moves[e.key][0]);
    c.y = round2(c.y + moves[e.key][1]);
    e.preventDefault();
    manualChanged();
  } else if (e.key === 'Delete' || e.key === 'Backspace') {
    e.preventDefault();
    manualAction('del');
  } else if (e.key === 'r' || e.key === 'R') {
    manualAction('rotate');
  } else if (e.key === 'd' || e.key === 'D') {
    e.preventDefault();
    manualAction('dup');
  } else if (e.key === 'Escape') {
    state.sel = null;
    draw();
  }
}

function manualSelInput(e) {
  const d = activeDoc();
  if (!d || state.sel === null) return;
  const c = curSide(d)[state.sel];
  const v = Number(e.target.value);
  if (!c || !Number.isFinite(v)) return;
  const b = numOr('bleed', 0);
  const [, H] = manualOf(d).sheet_mm;
  const [, fh] = footprint(d, c);
  if (e.target.id === 'm-left') c.x = round2(v - b);
  if (e.target.id === 'm-top') c.y = round2(H - v + b - fh);
  manualChanged();
}

// ---- page editor --------------------------------------------------------------------------
// Works on a list of what the new document will hold; thumbnails come from the source tabs and
// nothing is processed until "Terapkan", when the engine moves, copies and turns the real page
// objects (links, comments and bookmarks go with them) and writes the result as a new tab.

let orgSeq = 0;
const thumbs = new Map(); // `${docId}:${page}` -> data URL | Promise

function orgOf(d) {
  if (!d.org) orgReset(d);
  return d.org;
}

function orgReset(d) {
  d.org = { items: d.sizes.map((_, i) => ({ key: ++orgSeq, doc: d.id, page: i, rot: 0 })), sel: new Set(), anchor: null, undo: [], clip: [] };
}

function orgSnapshot(o) {
  o.undo.push(o.items.map((x) => ({ ...x })));
  if (o.undo.length > 50) o.undo.shift();
}

function orgSize(it) {
  if (it.blank) return it.blank;
  const src = state.docs.find((x) => x.id === it.doc);
  return (src && src.sizes[it.page]) || [210, 297];
}

function thumb(docId, page) {
  const key = `${docId}:${page}`;
  if (thumbs.has(key)) return thumbs.get(key);
  const src = state.docs.find((x) => x.id === docId);
  if (!src) return null;
  const p = (async () => {
    const doc = await pdfDoc(src.bytes);
    const pg = await doc.getPage(page + 1);
    const vp1 = pg.getViewport({ scale: 1 });
    const sc = 220 / Math.max(vp1.width, vp1.height);
    const vp = pg.getViewport({ scale: sc });
    const c = document.createElement('canvas');
    c.width = Math.ceil(vp.width);
    c.height = Math.ceil(vp.height);
    await pg.render({ canvasContext: c.getContext('2d'), viewport: vp }).promise;
    const url = c.toDataURL('image/jpeg', 0.8);
    thumbs.set(key, url);
    return url;
  })();
  thumbs.set(key, p);
  p.catch(() => thumbs.delete(key));
  return p;
}

function renderOrg() {
  const d = activeDoc();
  if (!d) return;
  const o = orgOf(d);
  const from = $('o-from');
  const opts = state.docs.map((x) => `<option value="${x.id}" ${x.id === Number(from.value) ? 'selected' : ''}>${esc(x.name)} (${x.sizes.length} hal.)</option>`).join('');
  if (from.innerHTML !== opts) from.innerHTML = opts;
  const grid = $('orggrid');
  grid.innerHTML = o.items.map((it, i) => {
    const [w, h] = orgSize(it);
    const turned = it.rot % 180 !== 0;
    const label = it.blank ? 'kosong' : it.doc === d.id ? `hal. ${it.page + 1}` : `${esc((state.docs.find((x) => x.id === it.doc) || { name: '?' }).name)} · ${it.page + 1}`;
    return `<figure class="pg ${o.sel.has(i) ? 'on' : ''}" draggable="true" data-i="${i}">
      <div class="pgbox"><div class="pgimg ${it.blank ? 'blank' : ''}" style="${thumbStyle(w, h, turned)}transform:rotate(${it.rot}deg)" data-key="${it.blank ? '' : `${it.doc}:${it.page}`}"></div></div>
      <figcaption><b>${i + 1}</b> <span>${label}${it.rot ? ` · ${it.rot > 0 ? '⟳' : '⟲'}${Math.abs(it.rot)}°` : ''}</span></figcaption>
    </figure>`;
  }).join('');
  $('orginfo').textContent = o.sel.size ? `${o.sel.size} dipilih` : '';
  // Fill thumbnails, cached per source page.
  grid.querySelectorAll('.pgimg[data-key]').forEach((el) => {
    const key = el.dataset.key;
    if (!key) return;
    const [docId, page] = key.split(':').map(Number);
    const t = thumb(docId, page);
    const set = (url) => { el.style.backgroundImage = `url(${url})`; };
    if (typeof t === 'string') set(t);
    else if (t) t.then(set).catch(() => {});
  });
}

/** Size of a thumbnail so that, turned, it fits a 112 x 150 box. */
function thumbStyle(w, h, turned) {
  const [bw, bh] = turned ? [h, w] : [w, h];
  const k = Math.min(112 / bw, 150 / bh);
  return `width:${Math.round(w * k)}px;height:${Math.round(h * k)}px;`;
}

function orgSelected(o) {
  return [...o.sel].sort((a, b) => a - b);
}

function orgAction(what) {
  const d = activeDoc();
  if (!d) return;
  const o = orgOf(d);
  const sel = orgSelected(o);
  const after = sel.length ? sel[sel.length - 1] + 1 : o.items.length;
  if (what === 'undo') {
    if (o.undo.length) { o.items = o.undo.pop(); o.sel.clear(); }
  } else if (what === 'reset') {
    orgSnapshot(o);
    const undo = o.undo;
    orgReset(d);
    d.org.undo = undo;
  } else if (what === 'blank') {
    orgSnapshot(o);
    const ref = o.items[sel.length ? sel[sel.length - 1] : o.items.length - 1];
    const [w, h] = ref ? orgSize(ref) : [210, 297];
    const turned = ref && ref.rot % 180 !== 0;
    o.items.splice(after, 0, { key: ++orgSeq, blank: turned ? [h, w] : [w, h], rot: 0 });
    o.sel = new Set([after]);
  } else if (what === 'insert') {
    const src = state.docs.find((x) => x.id === Number($('o-from').value));
    if (!src) return;
    const pages = parseRange($('o-range').value, src.sizes.length);
    if (!pages) return say('Halaman tidak valid. Contoh: 1-3,5', true);
    orgSnapshot(o);
    o.items.splice(after, 0, ...pages.map((p) => ({ key: ++orgSeq, doc: src.id, page: p, rot: 0 })));
    o.sel = new Set(pages.map((_, k) => after + k));
  } else if (what === 'paste') {
    if (!o.clip.length) return;
    orgSnapshot(o);
    o.items.splice(after, 0, ...o.clip.map((x) => ({ ...x, key: ++orgSeq })));
    o.sel = new Set(o.clip.map((_, k) => after + k));
  } else {
    if (!sel.length) return say('Pilih halaman dulu.', true);
    if (what === 'copy') {
      o.clip = sel.map((i) => ({ ...o.items[i] }));
      return say(`${sel.length} halaman disalin. Ctrl+V untuk tempel sesudah pilihan.`, false, true);
    }
    orgSnapshot(o);
    if (what === 'rotl' || what === 'rotr') {
      for (const i of sel) o.items[i].rot = (((o.items[i].rot + (what === 'rotr' ? 90 : -90) + 180) % 360) + 360) % 360 - 180 || 0;
    } else if (what === 'dup') {
      const copies = sel.map((i) => ({ ...o.items[i], key: ++orgSeq }));
      o.items.splice(after, 0, ...copies);
      o.sel = new Set(copies.map((_, k) => after + k));
    } else if (what === 'del') {
      if (sel.length === o.items.length) { o.undo.pop(); return say('Minimal satu halaman harus tersisa.', true); }
      o.items = o.items.filter((_, i) => !o.sel.has(i));
      o.sel.clear();
    } else if (what === 'left' || what === 'right') {
      const to = what === 'left' ? Math.max(0, sel[0] - 1) : Math.min(o.items.length - sel.length, sel[0] + 1);
      orgMove(o, sel, to);
    }
  }
  draw();
}

/** Move the selected items (kept in order) so they start at `to` in the resulting list. */
function orgMove(o, sel, to) {
  const moving = sel.map((i) => o.items[i]);
  const rest = o.items.filter((_, i) => !o.sel.has(i));
  const at = Math.max(0, Math.min(to, rest.length));
  rest.splice(at, 0, ...moving);
  o.items = rest;
  o.sel = new Set(moving.map((_, k) => at + k));
}

/** "1-3,5" -> [0, 1, 2, 4]; empty or "semua" -> every page; null when invalid. */
function parseRange(t, n) {
  t = String(t || '').trim().toLowerCase();
  if (!t || t === 'semua' || t === 'all') return [...Array(n).keys()];
  const out = [];
  for (const part of t.split(',')) {
    const m = part.trim().match(/^(\d+)(?:\s*-\s*(\d+))?$/);
    if (!m) return null;
    const a = Number(m[1]), b = m[2] ? Number(m[2]) : a;
    if (a < 1 || b > n || a > b) return null;
    for (let p = a; p <= b; p++) out.push(p - 1);
  }
  return out.length ? out : null;
}

let orgDrag = null;
function orgWire() {
  const grid = $('orggrid');
  grid.addEventListener('click', (e) => {
    const f = e.target.closest('.pg');
    const d = activeDoc();
    if (!d) return;
    const o = orgOf(d);
    if (!f) { o.sel.clear(); return draw(); }
    const i = Number(f.dataset.i);
    if (e.shiftKey && o.anchor !== null) {
      const [a, b] = [Math.min(o.anchor, i), Math.max(o.anchor, i)];
      if (!(e.ctrlKey || e.metaKey)) o.sel.clear();
      for (let k = a; k <= b; k++) o.sel.add(k);
    } else if (e.ctrlKey || e.metaKey) {
      if (o.sel.has(i)) o.sel.delete(i); else o.sel.add(i);
      o.anchor = i;
    } else {
      o.sel = new Set([i]);
      o.anchor = i;
    }
    grid.focus({ preventScroll: true });
    draw();
  });
  grid.addEventListener('dragstart', (e) => {
    const f = e.target.closest('.pg');
    const d = activeDoc();
    if (!f || !d) return;
    const o = orgOf(d);
    const i = Number(f.dataset.i);
    if (!o.sel.has(i)) o.sel = new Set([i]);
    orgDrag = true;
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', 'pages');
  });
  grid.addEventListener('dragover', (e) => {
    if (!orgDrag) return;
    e.preventDefault();
    const f = e.target.closest('.pg');
    grid.querySelectorAll('.pg.dropL,.pg.dropR').forEach((x) => x.classList.remove('dropL', 'dropR'));
    if (f) {
      const r = f.getBoundingClientRect();
      f.classList.add(e.clientX < r.left + r.width / 2 ? 'dropL' : 'dropR');
    }
  });
  grid.addEventListener('drop', (e) => {
    if (!orgDrag) return;
    e.preventDefault();
    orgDrag = null;
    const d = activeDoc();
    if (!d) return;
    const o = orgOf(d);
    const f = e.target.closest('.pg');
    let slot = o.items.length;
    if (f) {
      const r = f.getBoundingClientRect();
      slot = Number(f.dataset.i) + (e.clientX < r.left + r.width / 2 ? 0 : 1);
    }
    const sel = orgSelected(o);
    // Position among the pages that stay: slots before it that are moving don't count.
    const to = slot - sel.filter((i) => i < slot).length;
    orgSnapshot(o);
    orgMove(o, sel, to);
    draw();
  });
  grid.addEventListener('dragend', () => {
    orgDrag = null;
    grid.querySelectorAll('.pg.dropL,.pg.dropR').forEach((x) => x.classList.remove('dropL', 'dropR'));
  });
  grid.addEventListener('keydown', (e) => {
    const k = e.key.toLowerCase();
    const mod = e.ctrlKey || e.metaKey;
    const d = activeDoc();
    if (!d) return;
    const map = { delete: 'del', backspace: 'del' };
    if (mod && k === 'a') { e.preventDefault(); orgOf(d).sel = new Set(orgOf(d).items.map((_, i) => i)); return draw(); }
    if (mod && k === 'z') { e.preventDefault(); return orgAction('undo'); }
    if (mod && k === 'c') { e.preventDefault(); return orgAction('copy'); }
    if (mod && k === 'v') { e.preventDefault(); return orgAction('paste'); }
    if (mod && k === 'd') { e.preventDefault(); return orgAction('dup'); }
    if (!mod && k === 'r') { e.preventDefault(); return orgAction(e.shiftKey ? 'rotl' : 'rotr'); }
    if (map[k]) { e.preventDefault(); return orgAction(map[k]); }
  });
  $('org').addEventListener('click', (e) => {
    const b = e.target.closest('[data-o]');
    if (b) orgAction(b.dataset.o);
  });
}

/** The page editor holds changes that are not in the tab's file yet. */
function orgDirty(d) {
  if (!d || !d.org) return false;
  const it = d.org.items;
  return it.length !== d.sizes.length || it.some((x, i) => x.blank || x.doc !== d.id || x.page !== i || x.rot);
}

/**
 * Leaving the page editor: pending changes become a new tab first, so every layout after this
 * reads the new page order. Returns false when that failed (the user stays in the editor).
 */
async function leavePages(nextView) {
  const d = activeDoc();
  if (state.view !== 'pages' || !orgDirty(d)) return true;
  const ok = await applyPages(nextView, true);
  return ok;
}

async function applyPages(nextView = 'doc', auto = false) {
  const d = activeDoc();
  if (!d || !state.engine) return false;
  const o = orgOf(d);
  const extras = [];
  const plan = o.items.map((it) => {
    if (it.blank) return { blank_mm: it.blank };
    let src = 0;
    if (it.doc !== d.id) {
      let k = extras.findIndex((x) => x.id === it.doc);
      if (k < 0) { const x = state.docs.find((z) => z.id === it.doc); if (!x) return null; extras.push(x); k = extras.length - 1; }
      src = k + 1;
    }
    return { src, page: it.page, rotate: it.rot };
  });
  if (plan.some((p) => !p)) { say('Ada halaman dari tab yang sudah ditutup. Hapus dulu halaman itu.', true); return false; }
  state.busy = true;
  draw();
  say('Menyusun halaman…');
  await new Promise((r) => setTimeout(r, 20));
  let note = null;
  let done = false;
  try {
    const total = extras.reduce((n, x) => n + x.bytes.length, 0);
    const buf = new Uint8Array(total);
    let at = 0;
    for (const x of extras) { buf.set(x.bytes, at); at += x.bytes.length; }
    const out = state.engine.assemblePages(d.bytes.slice(), buf, new Uint32Array(extras.map((x) => x.bytes.length)), JSON.stringify(plan));
    const nd = addDoc(`${d.name} › Edit halaman`, out, [...d.tags, 'EDIT']);
    if (nd) {
      // The edits now live in the new tab; the original tab starts clean again.
      orgReset(d);
      state.view = nextView;
      state.sheet = 0;
      done = true;
      note = [`${auto ? 'Urutan halaman baru dipakai: ' : ''}tab baru "${nd.name}" (${plan.length} halaman). Layout berikutnya memakai tab ini. Halaman yang dihapus benar-benar hilang dari file.`, false, true];
    }
  } catch (e) {
    note = ['Edit halaman gagal: ' + String(e.message || e), true];
  } finally {
    state.busy = false;
    recompute();
    if (note) say(...note);
  }
  return done;
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
$('modes').addEventListener('click', async (e) => {
  const b = e.target.closest('button[data-mode]');
  if (!b) return;
  if (state.view === 'pages') {
    if (!(await leavePages('layout'))) return;
    state.view = 'layout';
  }
  const prevMode = state.mode;
  const prevLayout = state.layout;
  state.mode = b.dataset.mode;
  if (state.mode === 'manual') {
    const d = activeDoc();
    if (d && !d.manual && prevLayout && ['repeat', 'sequential', 'cut-stack'].includes(prevMode)) d.manual = manualFromLayout(d, prevLayout);
    state.sel = null;
    state.sheet = 0;
  }
  $('modes').querySelectorAll('button').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
  if (state.view === 'doc') state.view = 'layout';
  renderOptions();
});
$('views').addEventListener('click', async (e) => {
  const b = e.target.closest('button[data-view]');
  if (!b) return;
  if (b.dataset.view !== 'pages' && !(await leavePages(b.dataset.view))) return;
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
    state.sel = null;
    renderTabs();
    showDocInfo();
    if (state.mode === 'manual') renderOptions();
    else recompute();
  }
});
$('prev').addEventListener('click', () => { state.sheet = Math.max(0, state.sheet - 1); draw(); });
$('next').addEventListener('click', () => { state.sheet = Math.min(pageCount() - 1, state.sheet + 1); draw(); });
$('apply').addEventListener('click', () => (state.view === 'pages' ? applyPages() : apply()));
$('ack').addEventListener('change', draw);
$('pf').addEventListener('click', (e) => {
  if (e.target.closest('#do-outline')) outlineActive();
  if (e.target.closest('#do-inks')) inksActive();
});
$('pf').addEventListener('change', (e) => {
  if (!e.target.closest('.inkact')) return;
  const b = $('do-inks');
  if (b) b.disabled = !Object.keys(inkActions()).length;
});
$('opts').addEventListener('click', (e) => {
  const b = e.target.closest('[data-m]');
  if (b) manualAction(b.dataset.m);
});
$('opts').addEventListener('input', (e) => { if (e.target.id === 'm-left' || e.target.id === 'm-top') manualSelInput(e); });
$('cv').addEventListener('pointerdown', manualPointerDown);
$('cv').addEventListener('pointermove', manualPointerMove);
$('cv').addEventListener('pointerup', manualPointerUp);
$('cv').addEventListener('pointercancel', manualPointerUp);
document.addEventListener('keydown', manualKey);
$('download').addEventListener('click', async () => { if (await leavePages('pages')) save(false); });
$('saveas').addEventListener('click', async () => { if (await leavePages('pages')) save(true); });

orgWire();
renderOptions();
loadEngine().then(draw);
