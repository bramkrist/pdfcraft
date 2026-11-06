/* tslint:disable */
/* eslint-disable */

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
 * `pageSizes(pdfBytes)`: `[[w, h], ...]` in mm, as JSON.
 */
export function pageSizes(pdf: Uint8Array): string;

/**
 * The engine version, to show in the UI and record with a job.
 */
export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly impose: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly layout: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly layoutPdf: (a: number, b: number, c: number, d: number) => [number, number, number, number];
    readonly pageSizes: (a: number, b: number) => [number, number, number, number];
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
