/* tslint:disable */
/* eslint-disable */

/**
 * A changed file and the report of what changed, as JSON.
 */
export class Converted {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    readonly bytes: Uint8Array;
    readonly report: string;
}

/**
 * `assemblePages(pdfBytes, extrasBytes, extraLengths, planJson)`: the page editor's new
 * document. `extrasBytes` holds the other files one after another, `extraLengths` their sizes.
 */
export function assemblePages(pdf: Uint8Array, extras: Uint8Array, extra_lengths: Uint32Array, plan_json: string): Uint8Array;

/**
 * `impose(pdfBytes, jobJson)`: the imposed PDF bytes.
 */
export function impose(pdf: Uint8Array, job_json: string): Uint8Array;

/**
 * `layout(pagesJson, jobJson)`: pages as `[[w, h], ...]` in mm; returns the layout as JSON.
 */
export function layout(pages_json: string, job_json: string): string;

/**
 * `layoutPdf(pdfBytes, jobJson)`: the layout of a real file, as JSON.
 */
export function layoutPdf(pdf: Uint8Array, job_json: string): string;

/**
 * `manageInks(pdfBytes, actionsJson)`: `{ bytes, report }` with spots merged or made process.
 */
export function manageInks(pdf: Uint8Array, actions_json: string): Converted;

/**
 * `outlineText(pdfBytes)`: `{ bytes, report }`, every embedded-font glyph as a path.
 */
export function outlineText(pdf: Uint8Array): Converted;

/**
 * `pageSizes(pdfBytes)`: `[[w, h], ...]` in mm, as JSON.
 */
export function pageSizes(pdf: Uint8Array): string;

/**
 * `preflight(pdfBytes)`: `{ overall, checks: [{id, title, status, message}], report }` as JSON.
 */
export function preflight(pdf: Uint8Array): string;

/**
 * The engine version, to show in the UI and record with a job.
 */
export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_converted_free: (a: number, b: number) => void;
    readonly assemblePages: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number, number];
    readonly converted_bytes: (a: number) => [number, number];
    readonly converted_report: (a: number) => [number, number];
    readonly impose: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly layout: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly layoutPdf: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly manageInks: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly outlineText: (a: number, b: number) => [number, number, number];
    readonly pageSizes: (a: number, b: number) => [number, number, number, number];
    readonly preflight: (a: number, b: number) => [number, number, number, number];
    readonly version: () => [number, number];
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
