// imposeLib.js — simulasi imposisi di POS PIXELINDIE.
// Mesin hitung = pdfcraft-impose (WebAssembly, sama dengan impose.pixelindie.com), dipanggil
// lewat engine.layout(pagesJson, jobJson). File ini: menyusun job dari isian CGO, menghitung
// total lembar cetak, dan menggambar layout sebagai garis teks untuk nota thermal 48 kolom.
//
// impose_meta (pixelindie_order_items.impose_meta):
// { v: 1, name, kind: 'single'|'book', parts: [Part], engine, by, at }
// Part: { label, job, pages: { size_mm: [w,h], count }, material, sides, qty, insheet,
//         finished_mm, result: { sheet_count, up_per_sheet, sheet_mm, press_sheets, sides_per_copy },
//         layout_ascii: [string] }

export const SHEETS = {
  'A3+ 32×48': [320, 480],
  '33×48': [330, 480],
  'SRA3': [320, 450],
  'A3': [297, 420],
  'A4': [210, 297],
};

const r1 = (n) => Math.round(n * 10) / 10;

/** Sheet in the orientation asked: landscape = wider than tall. */
export function orient([w, h], landscape) {
  return landscape === w > h ? [w, h] : [h, w];
}

/**
 * Total press sheets for one part.
 * - repeat: every page needs ceil(qty / up) sheets.
 * - others: sheet sides of one copy (or set), halved for duplex/booklet, times qty.
 */
export function pressSheets(job, layout, pagesCount, qty) {
  const q = Math.max(1, Math.floor(Number(qty) || 1));
  if (!layout || !layout.sheet_count) return 0;
  if (job.mode === 'repeat') {
    const up = Math.max(1, layout.up_per_sheet);
    return Math.ceil(q / up) * Math.max(1, pagesCount);
  }
  const twoSided = job.mode === 'booklet' || !!job.duplex;
  const perCopy = twoSided ? Math.ceil(layout.sheet_count / 2) : layout.sheet_count;
  return perCopy * q;
}

/**
 * Book → two parts (cover + isi). Lengths in mm.
 * spec: { finished: [w,h], pages, spine, bleed, binding: 'saddle'|'perfect', signature,
 *         coverSheet: [w,h], isiSheet: [w,h], coverMaterial, isiMaterial, coverSides, isiSides,
 *         qty, gutter }
 */
export function bookParts(spec) {
  const [w, h] = spec.finished;
  const bleed = Number(spec.bleed) || 0;
  const spine = spec.binding === 'saddle' ? 0 : Number(spec.spine) || 0;
  const g = Number(spec.gutter) || 0;
  const coverW = 2 * w + spine + 2 * bleed;
  const coverH = h + 2 * bleed;
  const cover = {
    label: 'Cover Buku',
    pages: { size_mm: [r1(coverW), r1(coverH)], count: 1 },
    finished_mm: [r1(2 * w + spine), h],
    material: spec.coverMaterial || '',
    sides: spec.coverSides || '4/0',
    qty: spec.qty,
    job: { mode: 'repeat', sheet_mm: spec.coverSheet, margin_mm: 5, gutter_mm: [g, g], bleed_mm: bleed, marks: true },
    note: spine ? `Punggung ${r1(spine)} mm` : 'Tanpa punggung (steples)',
  };
  const isiPages = Math.max(1, Math.floor(Number(spec.pages) || 1));
  const isiJob = { mode: 'booklet', sheet_mm: spec.isiSheet, scale: false, align: 'spine' };
  if (spec.binding === 'perfect') {
    isiJob.binding = 'perfect';
    isiJob.signature = Number(spec.signature) || 16;
    isiJob.last_signature = 'multiple-of-four';
  } else {
    isiJob.binding = 'saddle';
  }
  const isi = {
    label: 'Isi Buku',
    pages: { size_mm: [w, h], count: isiPages },
    finished_mm: [w, h],
    material: spec.isiMaterial || '',
    sides: spec.isiSides || '4/4',
    qty: spec.qty,
    job: isiJob,
    note: spec.binding === 'perfect' ? `Lem panas · signature ${isiJob.signature} hal` : 'Steples tengah',
  };
  return [cover, isi];
}

/** Run the engine for one part; fills part.result and part.layout_ascii. */
export function computePart(engine, part, cols = 40) {
  const pages = Array.from({ length: part.pages.count }, () => part.pages.size_mm);
  const layout = JSON.parse(engine.layout(JSON.stringify(pages), JSON.stringify(part.job)));
  const first = layout.sheets[0];
  part.result = {
    sheet_count: layout.sheet_count,
    up_per_sheet: layout.up_per_sheet,
    sheet_mm: first ? first.size_mm.map(r1) : null,
    press_sheets: pressSheets(part.job, layout, part.pages.count, part.qty) + (Number(part.insheet) || 0),
  };
  const shown = part.job.mode === 'repeat' ? layout.sheets.slice(0, 1) : layout.sheets.slice(0, 2);
  part.layout_ascii = shown.flatMap((s, i) => [
    ...(shown.length > 1 ? [i === 0 ? 'Lembar 1 depan' : 'Lembar 1 belakang'] : []),
    ...asciiSheet(s, cols),
  ]);
  return layout;
}

/**
 * A sheet as box-drawing text: the sheet outline, every page cell, its page number.
 * `cols` is the width in characters; rows follow the sheet's proportions (a thermal character
 * is about twice as tall as it is wide).
 */
export function asciiSheet(sheet, cols = 40) {
  const [W, H] = sheet.size_mm;
  const C = Math.max(12, cols);
  const R = Math.max(5, Math.min(14, Math.round((C * H) / W / 2.4)));
  const grid = Array.from({ length: R }, () => Array(C).fill(' '));
  const put = (r, c, ch) => {
    if (r >= 0 && r < R && c >= 0 && c < C) {
      const cur = grid[r][c];
      grid[r][c] = (cur === '-' && ch === '|') || (cur === '|' && ch === '-') || cur === '+' ? '+' : ch;
    }
  };
  const X = (x) => Math.round((x / W) * (C - 1));
  const Y = (y) => Math.round(((H - y) / H) * (R - 1));
  const box = (x0, y0, x1, y1) => {
    const c0 = X(x0), c1 = X(x1), r0 = Y(y1), rr1 = Y(y0);
    for (let c = c0; c <= c1; c++) { put(r0, c, '-'); put(rr1, c, '-'); }
    for (let r = r0; r <= rr1; r++) { put(r, c0, '|'); put(r, c1, '|'); }
    put(r0, c0, '+'); put(r0, c1, '+'); put(rr1, c0, '+'); put(rr1, c1, '+');
    return { c0, c1, r0, r1: rr1 };
  };
  // Sheet edge: dots, so it is told apart from cut lines.
  for (let c = 0; c < C; c++) { grid[0][c] = '.'; grid[R - 1][c] = '.'; }
  for (let r = 0; r < R; r++) { grid[r][0] = '.'; grid[r][C - 1] = '.'; }
  const cells = sheet.cells.filter((c) => !c.mirrored);
  for (const cell of cells) {
    const b = box(...cell.rect_mm);
    const label = String(cell.page + 1);
    const mr = Math.round((b.r0 + b.r1) / 2);
    const mc = Math.round((b.c0 + b.c1) / 2 - (label.length - 1) / 2);
    if (b.c1 - b.c0 > label.length + 1 && b.r1 - b.r0 >= 2) {
      for (let i = 0; i < label.length; i++) grid[mr][mc + i] = label[i];
    }
  }
  return grid.map((row) => row.join('').replace(/\s+$/, ''));
}

/** Nota lines (usePrinter line objects) for one item's impose_meta: the operator's page 2. */
export function imposeNotaLines(meta, itemName, cols = 48) {
  if (!meta || !Array.isArray(meta.parts) || meta.parts.length === 0) return [];
  const L = [];
  const text = (t) => L.push({ type: 'text', text: String(t).slice(0, cols) });
  const kv = (k, v) => text(`${k.padEnd(11)}: ${v}`);
  L.push({ type: 'align', value: 'center' }, { type: 'bold', on: true }, { type: 'size', h: 1 });
  text('LAYOUT CETAK');
  L.push({ type: 'size', h: 0 });
  text(meta.name || '-');
  L.push({ type: 'bold', on: false }, { type: 'align', value: 'left' });
  if (itemName) text(itemName);
  L.push({ type: 'divider_heavy' });
  for (const p of meta.parts) {
    const res = p.result || {};
    L.push({ type: 'bold', on: true });
    text(p.label || 'Cetak');
    L.push({ type: 'bold', on: false });
    if (p.finished_mm) kv('Ukuran jadi', `${p.finished_mm[0]} x ${p.finished_mm[1]} mm`);
    kv('File/hal', `${p.pages.size_mm[0]} x ${p.pages.size_mm[1]} mm · ${p.pages.count} hal`);
    if (p.job.bleed_mm) kv('Bleed', `${p.job.bleed_mm} mm`);
    if (p.material) kv('Bahan', p.material);
    if (res.sheet_mm) kv('Kertas', `${res.sheet_mm[0]} x ${res.sheet_mm[1]} mm`);
    kv('Sisi', p.sides || '-');
    kv('Susunan', modeLabel(p.job) + (res.up_per_sheet ? ` · ${res.up_per_sheet} up` : ''));
    kv('Qty', String(p.qty ?? '-'));
    kv('Lembar', `${res.press_sheets ?? '-'}${p.insheet ? ` (insheet ${p.insheet})` : ''}`);
    if (p.note) kv('Catatan', p.note);
    L.push({ type: 'feed', n: 1 });
    for (const row of p.layout_ascii || []) text(row);
    L.push({ type: 'divider_light' });
  }
  L.push({ type: 'cut' });
  return L;
}

export function modeLabel(job) {
  switch (job.mode) {
    case 'repeat': return 'Step & repeat';
    case 'sequential': return 'N-up';
    case 'cut-stack': return `Unique collate${job.copies > 1 ? ` x${job.copies}` : ''}`;
    case 'booklet': return job.binding === 'perfect' ? `Booklet sig.${job.signature}` : 'Booklet steples';
    case 'bleed': return 'Bleed';
    default: return job.mode;
  }
}
