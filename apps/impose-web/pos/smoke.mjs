import fs from 'fs';
import init, * as E from './pdfcraft_impose_web.js' /* from the impose-web-dist branch */;
import { bookParts, computePart, imposeNotaLines, SHEETS, orient } from './imposeLib.js';
await init({ module_or_path: fs.readFileSync('./pdfcraft_impose_web_bg.wasm') });
console.log('version', E.version());
const src = fs.readFileSync(process.argv[2]);
console.log('sizes', E.pageSizes(src));
const job = { mode:'repeat', sheet_mm:[480,320], margin_mm:0, cols:2, rows:2 };
const t0=Date.now(); const out = E.impose(src, JSON.stringify(job)); console.log('imposed bytes', out.length, Date.now()-t0,'ms');

// unique collate example
const L = JSON.parse(E.layout(JSON.stringify(Array(16).fill([100,100])), JSON.stringify({mode:'cut-stack',copies:2,sheet_mm:[400,200],margin_mm:0,cols:4,rows:2})));
console.log('cutstack sheets', L.sheet_count, 'up', L.up_per_sheet);
// book: A5 48 pages, perfect
const parts = bookParts({ finished:[148,210], pages:48, spine:4, bleed:3, binding:'perfect', signature:16, coverSheet: orient(SHEETS['A3+ 32×48'], true), isiSheet: orient(SHEETS['A3+ 32×48'], true), coverMaterial:'Art Carton 260', isiMaterial:'HVS 80', coverSides:'4/0', isiSides:'4/4', qty:50, gutter:4 });
for (const p of parts) computePart(E, p, 44);
const meta = { v:1, name:'BUKU-PAULUS-A5', kind:'book', parts };
for (const l of imposeNotaLines(meta, 'CETAK BUKU A5 48 HAL')) if (l.type==='text') console.log(l.text); else if (l.type.startsWith('divider')) console.log('-'.repeat(48)); else if (l.type==='cut') console.log('--- cut ---');
