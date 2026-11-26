// Images -> PDF for the "Gabung" panel (impose.pixelindie.com). No dependencies.
//
// Fidelity: a JPEG is embedded byte for byte (DCTDecode), so its pixels and colours are not touched.
// Anything else (PNG, WebP, ...) is decoded by the browser and written as lossless 8-bit RGB (Flate);
// transparency is flattened onto white. A JPEG whose EXIF orientation is not "normal" goes through
// the browser path as well, so the page comes out upright.

const enc = new TextEncoder();

/** Width, height, components and Adobe flag of a baseline/progressive JPEG, plus EXIF orientation (1 if none). */
export function parseJpeg(b) {
  if (b.length < 4 || b[0] !== 0xff || b[1] !== 0xd8) return null;
  let i = 2;
  let sof = null;
  let adobe = false;
  let orientation = 1;
  while (i + 4 <= b.length) {
    if (b[i] !== 0xff) { i++; continue; }
    const m = b[i + 1];
    if (m === 0xff) { i++; continue; }
    if (m === 0xd8 || m === 0x01 || (m >= 0xd0 && m <= 0xd7)) { i += 2; continue; }
    if (m === 0xd9 || m === 0xda) break;
    const len = (b[i + 2] << 8) | b[i + 3];
    if (len < 2 || i + 2 + len > b.length) return null;
    const p = i + 4;
    if (m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc) {
      if (len < 8) return null;
      sof = { bits: b[p], h: (b[p + 1] << 8) | b[p + 2], w: (b[p + 3] << 8) | b[p + 4], n: b[p + 5] };
    } else if (m === 0xee && len >= 14 && b[p] === 0x41 && b[p + 1] === 0x64 && b[p + 2] === 0x6f && b[p + 3] === 0x62 && b[p + 4] === 0x65) {
      adobe = true;
    } else if (m === 0xe1 && len >= 16 && b[p] === 0x45 && b[p + 1] === 0x78 && b[p + 2] === 0x69 && b[p + 3] === 0x66) {
      orientation = exifOrientation(b, p + 6, i + 2 + len);
    }
    i += 2 + len;
  }
  if (!sof || !sof.w || !sof.h || sof.bits !== 8 || ![1, 3, 4].includes(sof.n)) return null;
  return { w: sof.w, h: sof.h, n: sof.n, adobe, orientation };
}

function exifOrientation(b, t, end) {
  if (t + 8 > end) return 1;
  const le = b[t] === 0x49 && b[t + 1] === 0x49;
  if (!le && !(b[t] === 0x4d && b[t + 1] === 0x4d)) return 1;
  const u16 = (o) => (le ? b[o] | (b[o + 1] << 8) : (b[o] << 8) | b[o + 1]);
  const u32 = (o) => (le ? (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0 : ((b[o] << 24) | (b[o + 1] << 16) | (b[o + 2] << 8) | b[o + 3]) >>> 0);
  const ifd = t + u32(t + 4);
  if (ifd + 2 > end) return 1;
  const n = u16(ifd);
  for (let k = 0; k < n; k++) {
    const e = ifd + 2 + k * 12;
    if (e + 12 > end) return 1;
    if (u16(e) === 0x0112) {
      const v = u16(e + 8);
      return v >= 1 && v <= 8 ? v : 1;
    }
  }
  return 1;
}

const ptOfMm = (mm) => (mm * 72) / 25.4;
export const PAGE_MM = { A4: [210, 297], A3: [297, 420], A5: [148, 210] };

/**
 * Where the image goes on the page, in PDF points.
 * opts: { page: 'fit' | 'A4' | 'A3' | 'A5', dpi, marginMm }
 * 'fit' makes the page the size of the picture at `dpi`; a fixed size picks portrait or landscape to match the picture.
 */
export function placement(wPx, hPx, opts = {}) {
  const dpi = opts.dpi > 0 ? opts.dpi : 300;
  const margin = ptOfMm(Math.max(0, opts.marginMm || 0));
  if (!opts.page || opts.page === 'fit') {
    const w = (wPx * 72) / dpi;
    const h = (hPx * 72) / dpi;
    return { pageW: w + 2 * margin, pageH: h + 2 * margin, x: margin, y: margin, w, h };
  }
  const [a, b] = PAGE_MM[opts.page] || PAGE_MM.A4;
  const land = wPx > hPx;
  const pageW = ptOfMm(land ? b : a);
  const pageH = ptOfMm(land ? a : b);
  const boxW = Math.max(1, pageW - 2 * margin);
  const boxH = Math.max(1, pageH - 2 * margin);
  const k = Math.min(boxW / wPx, boxH / hPx);
  const w = wPx * k;
  const h = hPx * k;
  return { pageW, pageH, x: (pageW - w) / 2, y: (pageH - h) / 2, w, h };
}

const n3 = (v) => String(Math.round(v * 1000) / 1000);

/**
 * One-page PDF around an already-encoded image.
 * img: { w, h, data: Uint8Array, filter: 'DCTDecode' | 'FlateDecode', cs: 'DeviceGray' | 'DeviceRGB' | 'DeviceCMYK', invert?: boolean }
 */
export function pdfFromImage(img, opts = {}) {
  const pl = placement(img.w, img.h, opts);
  const content = `q ${n3(pl.w)} 0 0 ${n3(pl.h)} ${n3(pl.x)} ${n3(pl.y)} cm /Im0 Do Q\n`;
  const parts = [];
  const offsets = [];
  let len = 0;
  const push = (x) => {
    const u = typeof x === 'string' ? enc.encode(x) : x;
    parts.push(u);
    len += u.length;
  };
  push('%PDF-1.4\n%\xe2\xe3\xcf\xd3\n'); // the high-bit comment marks the file as binary
  const obj = (n, body) => {
    offsets[n] = len;
    push(`${n} 0 obj\n`);
    push(body);
    push('\nendobj\n');
  };
  obj(1, '<< /Type /Catalog /Pages 2 0 R >>');
  obj(2, '<< /Type /Pages /Kids [3 0 R] /Count 1 >>');
  obj(3, `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${n3(pl.pageW)} ${n3(pl.pageH)}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>`);
  obj(4, `<< /Length ${content.length} >>\nstream\n${content}endstream`);
  offsets[5] = len;
  push(`5 0 obj\n<< /Type /XObject /Subtype /Image /Width ${img.w} /Height ${img.h} /ColorSpace /${img.cs} /BitsPerComponent 8 /Filter /${img.filter}${img.invert ? ' /Decode [1 0 1 0 1 0 1 0]' : ''} /Length ${img.data.length} >>\nstream\n`);
  push(img.data);
  push('\nendstream\nendobj\n');
  const xref = len;
  let x = 'xref\n0 6\n0000000000 65535 f \n';
  for (let n = 1; n <= 5; n++) x += `${String(offsets[n]).padStart(10, '0')} 00000 n \n`;
  push(`${x}trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  const out = new Uint8Array(len);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
}

async function deflate(raw) {
  const cs = new CompressionStream('deflate');
  const w = cs.writable.getWriter();
  w.write(raw);
  w.close();
  return new Uint8Array(await new Response(cs.readable).arrayBuffer());
}

/** Browser path: decode with the browser, flatten alpha onto white, write 8-bit RGB with Flate. */
async function viaCanvas(file) {
  const bmp = await createImageBitmap(file, { imageOrientation: 'from-image' });
  const w = bmp.width;
  const h = bmp.height;
  if (w * h > 120e6) throw new Error(`${file.name}: gambar terlalu besar (${w}×${h} px).`);
  const cv = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(w, h) : Object.assign(document.createElement('canvas'), { width: w, height: h });
  const ctx = cv.getContext('2d', { willReadFrequently: true });
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, w, h);
  ctx.drawImage(bmp, 0, 0);
  bmp.close();
  const rgba = ctx.getImageData(0, 0, w, h).data;
  const rgb = new Uint8Array(w * h * 3);
  for (let i = 0, o = 0; i < rgba.length; i += 4, o += 3) {
    rgb[o] = rgba[i];
    rgb[o + 1] = rgba[i + 1];
    rgb[o + 2] = rgba[i + 2];
  }
  return { w, h, data: await deflate(rgb), filter: 'FlateDecode', cs: 'DeviceRGB' };
}

/** A one-page PDF (Uint8Array) from an image File (JPEG, PNG, WebP, GIF, BMP, ...). */
export async function imageToPdf(file, opts = {}) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const j = parseJpeg(bytes);
  let img;
  if (j && j.orientation === 1) {
    img = { w: j.w, h: j.h, data: bytes, filter: 'DCTDecode', cs: j.n === 1 ? 'DeviceGray' : j.n === 3 ? 'DeviceRGB' : 'DeviceCMYK', invert: j.n === 4 && j.adobe };
  } else {
    img = await viaCanvas(file);
  }
  return pdfFromImage(img, opts);
}

export const isImageName = (n) => /\.(jpe?g|png|webp|gif|bmp)$/i.test(n);
